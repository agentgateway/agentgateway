use std::pin::Pin;
use std::task::{Context, Poll};

use ::http::HeaderMap;
use bytes::{Bytes, BytesMut};
use http_body::Body;
use pin_project_lite::pin_project;
use tokio_util::codec::{Decoder, Encoder};

use crate::*;

pin_project! {
	pub struct TransformedBody<D, E, F, T> {
		#[pin]
		body: http::Body,
		decoder: D,
		decode_buffer: BytesMut,
		buffered_trailers: Option<HeaderMap>,
		encoder: E,
		handler: F,
		finished: bool,
		_phantom: std::marker::PhantomData<T>,
	}
}

pub fn parser<D, E, F, I, T>(body: http::Body, decoder: D, encoder: E, handler: F) -> http::Body
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(D::Item) -> I + Send + 'static,
	I: IntoIterator<Item = T>,
	E: Encoder<T> + Send + 'static,
	E::Error: Send + Into<axum_core::BoxError> + 'static,
	T: Send + 'static,
{
	http::Body::new(TransformedBody {
		body,
		decoder,
		handler,
		decode_buffer: BytesMut::new(),
		buffered_trailers: None,
		encoder,
		finished: false,
		_phantom: std::marker::PhantomData,
	})
}

impl<D, E, F, I, T> Body for TransformedBody<D, E, F, T>
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	E: Encoder<T> + Send + 'static,
	E::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(D::Item) -> I + Send + 'static,
	I: IntoIterator<Item = T>,
{
	type Data = Bytes;
	type Error = http::Error;

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
		                  encode_buf: &mut BytesMut| {
			loop {
				let decode = if finished {
					decoder.decode_eof(buf)
				} else {
					decoder.decode(buf)
				};
				match decode {
					Ok(Some(decoded_item)) => {
						for transformed_item in (handler)(decoded_item) {
							match encoder.encode(transformed_item, encode_buf) {
								Ok(()) => {},
								Err(e) => return Err(http::Error::new(e)),
							}
						}
					},
					Ok(None) => {
						return Ok(());
					},
					Err(e) => {
						return Err(http::Error::new(e));
					},
				}
			}
		};

		// Try to decode and encode items from our buffer
		if let Err(e) = (try_decode)(
			*this.finished,
			this.decode_buffer,
			&mut *this.decoder,
			this.handler,
			&mut *this.encoder,
			&mut encode_buffer,
		) {
			return Poll::Ready(Some(Err(e)));
		}

		// If we have encoded data to send, send it
		if !encode_buffer.is_empty() {
			let data = encode_buffer.split_to(encode_buffer.len());
			return Poll::Ready(Some(Ok(http_body::Frame::data(data.freeze()))));
		}

		// We need more input data - poll the underlying body
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
			Some(Err(e)) => Poll::Ready(Some(Err(e))),
			None => {
				*this.finished = true;
				// Try one more decode/encode cycle
				match (try_decode)(
					*this.finished,
					this.decode_buffer,
					&mut *this.decoder,
					this.handler,
					&mut *this.encoder,
					&mut encode_buffer,
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
