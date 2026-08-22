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
		pending_error: Option<axum_core::Error>,
		_phantom: std::marker::PhantomData<T>,
	}
}

pub enum TransformEvent<T> {
	Item(T),
	Eof,
	Error,
}

fn encode_event<Input, E, F, I, T>(
	event: TransformEvent<Input>,
	handler: &mut F,
	encoder: &mut E,
	buffer: &mut BytesMut,
) -> Result<(), axum_core::Error>
where
	F: FnMut(TransformEvent<Input>) -> I,
	I: IntoIterator<Item = T>,
	E: Encoder<T>,
	E::Error: Into<axum_core::BoxError>,
{
	for item in handler(event) {
		encoder
			.encode(item, buffer)
			.map_err(axum_core::Error::new)?;
	}
	Ok(())
}

fn terminal_error_frame<Input, E, F, I, T>(
	error: axum_core::Error,
	handler: &mut F,
	encoder: &mut E,
	mut preceding_output: BytesMut,
	pending_error: &mut Option<axum_core::Error>,
) -> Result<http_body::Frame<Bytes>, axum_core::Error>
where
	F: FnMut(TransformEvent<Input>) -> I,
	I: IntoIterator<Item = T>,
	E: Encoder<T>,
	E::Error: Into<axum_core::BoxError>,
{
	let preceding_output_len = preceding_output.len();
	encode_event(
		TransformEvent::Error,
		handler,
		encoder,
		&mut preceding_output,
	)?;
	if preceding_output.len() == preceding_output_len {
		if !preceding_output.is_empty() {
			*pending_error = Some(error);
			return Ok(http_body::Frame::data(preceding_output.freeze()));
		}
		return Err(error);
	}
	Ok(http_body::Frame::data(preceding_output.freeze()))
}

pub fn parser<D, E, F, I, T>(body: AxumBody, decoder: D, encoder: E, handler: F) -> AxumBody
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(TransformEvent<D::Item>) -> I + Send + 'static,
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
		pending_error: None,
		_phantom: std::marker::PhantomData,
	})
}

impl<D, E, F, I, T> HttpBody for TransformedBody<D, E, F, T>
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	E: Encoder<T> + Send + 'static,
	E::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(TransformEvent<D::Item>) -> I + Send + 'static,
	I: IntoIterator<Item = T>,
{
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
		let mut this = self.project();
		if let Some(error) = this.pending_error.take() {
			return Poll::Ready(Some(Err(error)));
		}
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
						encode_event(
							TransformEvent::Item(decoded_item),
							handler,
							encoder,
							encode_buf,
						)?;
					},
					Ok(None) => {
						if finished {
							encode_event(TransformEvent::Eof, handler, encoder, encode_buf)?;
						}
						return Ok(());
					},
					Err(e) => {
						return Err(axum_core::Error::new(e));
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
			*this.finished = true;
			return Poll::Ready(Some(terminal_error_frame(
				e,
				this.handler,
				&mut *this.encoder,
				encode_buffer,
				this.pending_error,
			)));
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
			Some(Err(e)) => {
				*this.finished = true;
				Poll::Ready(Some(terminal_error_frame(
					e,
					this.handler,
					&mut *this.encoder,
					encode_buffer,
					this.pending_error,
				)))
			},
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
					Err(e) => Poll::Ready(Some(terminal_error_frame(
						e,
						this.handler,
						&mut *this.encoder,
						encode_buffer,
						this.pending_error,
					))),
				}
			},
		}
	}
}
