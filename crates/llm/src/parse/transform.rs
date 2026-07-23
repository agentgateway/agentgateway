use std::pin::Pin;
use std::task::{Context, Poll, ready};

use ::http::HeaderMap;
use axum_core::body::Body as AxumBody;
use bytes::{Bytes, BytesMut};
use http_body::Body as HttpBody;
use pin_project_lite::pin_project;
use tokio_util::codec::{Decoder, Encoder};

pin_project! {
	pub struct TransformedBody<D, E, F, T> {
		#[pin]
		body: AxumBody,
		decoder: D,
		decode_buffer: BytesMut,
		buffered_trailers: Option<HeaderMap>,
		encoder: E,
		handler: F,
		finished: bool,
		transport_failures_to_handler: bool,
		_phantom: std::marker::PhantomData<T>,
	}
}

pub fn parser<D, E, F, I, T>(body: AxumBody, decoder: D, encoder: E, handler: F) -> AxumBody
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(D::Item) -> I + Send + 'static,
	I: IntoIterator<Item = T>,
	E: Encoder<T> + Send + 'static,
	E::Error: Send + Into<axum_core::BoxError> + 'static,
	T: Send + 'static,
{
	let mut handler = handler;
	parser_inner(body, decoder, encoder, false, move |input| {
		let Ok(item) = input else {
			unreachable!("ordinary transforms propagate transport failures")
		};
		(handler(item), false)
	})
}

pub(crate) fn strict_parser<D, E, F, I, T>(
	body: AxumBody,
	decoder: D,
	encoder: E,
	handler: F,
) -> AxumBody
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(Result<D::Item, ()>) -> (I, bool) + Send + 'static,
	I: IntoIterator<Item = T>,
	E: Encoder<T> + Send + 'static,
	E::Error: Send + Into<axum_core::BoxError> + 'static,
	T: Send + 'static,
{
	parser_inner(body, decoder, encoder, true, handler)
}

fn parser_inner<D, E, F, I, T>(
	body: AxumBody,
	decoder: D,
	encoder: E,
	transport_failures_to_handler: bool,
	handler: F,
) -> AxumBody
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(Result<D::Item, ()>) -> (I, bool) + Send + 'static,
	I: IntoIterator<Item = T>,
	E: Encoder<T> + Send + 'static,
	E::Error: Send + Into<axum_core::BoxError> + 'static,
	T: Send + 'static,
{
	AxumBody::new(TransformedBody {
		body,
		decoder,
		handler,
		decode_buffer: BytesMut::new(),
		buffered_trailers: None,
		encoder,
		finished: false,
		transport_failures_to_handler,
		_phantom: std::marker::PhantomData,
	})
}

impl<D, E, F, I, T> HttpBody for TransformedBody<D, E, F, T>
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	E: Encoder<T> + Send + 'static,
	E::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(Result<D::Item, ()>) -> (I, bool) + Send + 'static,
	I: IntoIterator<Item = T>,
{
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
		let mut this = self.project();
		// If we're finished and have no more data, we're done
		if *this.finished {
			if let Some(trailer) = std::mem::take(this.buffered_trailers) {
				// If there is no more data, send any trailers
				return Poll::Ready(Some(Ok(http_body::Frame::trailers(trailer))));
			}
			return Poll::Ready(None);
		}

		let mut encode_buffer = BytesMut::new();

		let try_decode = |finished: bool,
		                  buf: &mut BytesMut,
		                  decoder: &mut D,
		                  handler: &mut F,
		                  encoder: &mut E,
		                  encode_buf: &mut BytesMut,
		                  transport_failures_to_handler: bool,
		                  stop: &mut bool| {
			loop {
				let decode = if finished {
					decoder.decode_eof(buf)
				} else {
					decoder.decode(buf)
				};
				match decode {
					Ok(Some(decoded_item)) => {
						let (items, terminate) = (handler)(Ok(decoded_item));
						let encoded_before = encode_buf.len();
						for transformed_item in items {
							match encoder.encode(transformed_item, encode_buf) {
								Ok(()) => {},
								Err(e) => return Err(axum_core::Error::new(e)),
							}
						}
						if terminate {
							*stop = true;
							return Ok(());
						}
						if transport_failures_to_handler && !finished && encode_buf.len() > encoded_before {
							return Ok(());
						}
					},
					Ok(None) => {
						return Ok(());
					},
					Err(e) => {
						if !transport_failures_to_handler {
							return Err(axum_core::Error::new(e));
						}
						let (items, _) = (handler)(Err(()));
						for transformed_item in items {
							encoder
								.encode(transformed_item, encode_buf)
								.map_err(axum_core::Error::new)?;
						}
						*stop = true;
						return Ok(());
					},
				}
			}
		};

		// Try to decode and encode items from our buffer
		let finished = *this.finished;
		if let Err(e) = (try_decode)(
			finished,
			this.decode_buffer,
			&mut *this.decoder,
			this.handler,
			&mut *this.encoder,
			&mut encode_buffer,
			*this.transport_failures_to_handler,
			this.finished,
		) {
			return Poll::Ready(Some(Err(e)));
		}

		// If we have encoded data to send, send it
		if !encode_buffer.is_empty() {
			let data = encode_buffer.split_to(encode_buffer.len());
			return Poll::Ready(Some(Ok(http_body::Frame::data(data.freeze()))));
		}

		// We need more input data - poll the underlying body
		if *this.finished {
			return Poll::Ready(None);
		}

		let res = ready!(this.body.as_mut().poll_frame(cx));
		match res {
			Some(Ok(frame)) => {
				if let Some(data) = frame.data_ref() {
					this.decode_buffer.extend_from_slice(data);
				}
				if let Ok(trailer) = frame.into_trailers() {
					*this.buffered_trailers = Some(trailer);
				}
				// Continue processing - don't pass through the original frame
				cx.waker().wake_by_ref();
				Poll::Pending
			},
			Some(Err(e)) => {
				if !*this.transport_failures_to_handler {
					return Poll::Ready(Some(Err(e)));
				}
				let (items, _) = (this.handler)(Err(()));
				for transformed_item in items {
					if let Err(e) = this.encoder.encode(transformed_item, &mut encode_buffer) {
						return Poll::Ready(Some(Err(axum_core::Error::new(e))));
					}
				}
				*this.finished = true;
				if encode_buffer.is_empty() {
					Poll::Ready(None)
				} else {
					Poll::Ready(Some(Ok(http_body::Frame::data(encode_buffer.freeze()))))
				}
			},
			None => {
				*this.finished = true;
				// Try one more decode/encode cycle
				match (try_decode)(
					true,
					this.decode_buffer,
					&mut *this.decoder,
					this.handler,
					&mut *this.encoder,
					&mut encode_buffer,
					*this.transport_failures_to_handler,
					this.finished,
				) {
					Ok(_) => {
						if !encode_buffer.is_empty() {
							// If there is more data to encode, send it
							let data = encode_buffer.split_to(encode_buffer.len());
							Poll::Ready(Some(Ok(http_body::Frame::data(data.freeze()))))
						} else if let Some(trailer) = std::mem::take(this.buffered_trailers) {
							// If there is no more data, send any trailers
							Poll::Ready(Some(Ok(http_body::Frame::trailers(trailer))))
						} else {
							// Else return we are done.
							Poll::Ready(None)
						}
					},
					Err(e) => Poll::Ready(Some(Err(e))),
				}
			},
		}
	}
}
