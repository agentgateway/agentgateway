use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use http_body::Body;
use pin_project_lite::pin_project;
use tokio_util::codec::Decoder;

use crate::*;

pin_project! {
	pub struct PassthroughBody<D, F> {
		#[pin]
		body: http::Body,
		decoder: D,
		decode_buffer: BytesMut,
		handler: F,
		finished: bool,
	}
}

pub fn parser<D, F>(body: http::Body, decoder: D, handler: F) -> http::Body
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(D::Item) + Send + 'static,
{
	http::Body::new(PassthroughBody {
		body,
		decoder,
		handler,
		decode_buffer: BytesMut::new(),
		finished: false,
	})
}

impl<D, F> Body for PassthroughBody<D, F>
where
	D: Decoder + Send + 'static,
	D::Error: Send + Into<axum_core::BoxError> + 'static,
	F: FnMut(D::Item) + Send + 'static,
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
			return Poll::Ready(None);
		}

		let try_decode = |finished: bool, buf: &mut BytesMut, decoder: &mut D, handler: &mut F| {
			loop {
				let decode = if finished {
					decoder.decode_eof(buf)
				} else {
					decoder.decode(buf)
				};
				match decode {
					Ok(Some(decoded_item)) => {
						(handler)(decoded_item);
					},
					Ok(None) => {
						// Nothing more to decode!
						return Ok(());
					},
					Err(e) => {
						return Err(http::Error::new(e));
					},
				}
			}
		};

		// Try to decode items from our buffer
		if let Err(e) = (try_decode)(
			*this.finished,
			this.decode_buffer,
			&mut *this.decoder,
			this.handler,
		) {
			return Poll::Ready(Some(Err(e)));
		}
		// We need more input data - poll the underlying body
		let res = ready!(this.body.as_mut().poll_frame(cx));
		let frame_to_send = match res {
			Some(Ok(frame)) => {
				if let Some(data) = frame.data_ref() {
					this.decode_buffer.extend_from_slice(data);
				}
				Some(Ok(frame))
			},
			Some(Err(e)) => {
				return Poll::Ready(Some(Err(e)));
			},
			None => {
				*this.finished = true;
				None
			},
		};

		match (try_decode)(
			*this.finished,
			this.decode_buffer,
			&mut *this.decoder,
			this.handler,
		) {
			Ok(_) => Poll::Ready(frame_to_send),
			Err(e) => Poll::Ready(Some(Err(e))),
		}
	}
}
