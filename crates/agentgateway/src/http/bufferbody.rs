use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes};
use http_body::{Body as HttpBody, Frame, SizeHint};
use parking_lot::Mutex;
use pin_project_lite::pin_project;

use crate::http::buflist::BufList;
use crate::*;

#[cfg(test)]
#[path = "bufferbody_tests.rs"]
mod tests;

#[derive(Clone, Debug, Default)]
pub struct BufferedBodyHandle {
	inner: Arc<Mutex<BufferedBodyHandleInner>>,
}

#[derive(Debug, Default)]
struct BufferedBodyHandleInner {
	bytes: Option<Bytes>,
}

impl BufferedBodyHandle {
	/// Returns the complete buffered data once the body has reached EOF.
	///
	/// This is `None` while the body is still buffering, or if buffering fails before EOF.
	pub fn bytes(&self) -> Option<Bytes> {
		self.inner.lock().bytes.clone()
	}

	fn complete(&self, bytes: Bytes) {
		self.inner.lock().bytes = Some(bytes);
	}
}

pin_project! {
	/// Buffers all data frames from an HTTP body and emits them as one final data frame.
	///
	/// This is useful for protocols that need to withhold body bytes until EOF, while still
	/// treating the result as a normal [`HttpBody`] stream once buffering is complete.
	pub struct BufferedBody<B = crate::http::Body> {
		#[pin]
		inner: B,
		buffer: BufList,
		trailers: Option<::http::HeaderMap>,
		state: BufferedBodyState,
		handle: BufferedBodyHandle,
		limit: usize,
		buffered: usize,
	}
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum BufferedBodyState {
	/// Still draining the inner body. During this phase we only accumulate data; callers should
	/// not see body frames until the inner body reaches EOF.
	Buffering,
	/// Data has been emitted, and a trailer frame observed while buffering still needs replay.
	EmitTrailers,
	/// The synthetic buffered body is fully consumed.
	Done,
}

impl<B> BufferedBody<B> {
	pub fn new(inner: B) -> (Self, BufferedBodyHandle) {
		Self::new_with_limit(inner, usize::MAX)
	}

	pub fn new_with_limit(inner: B, limit: usize) -> (Self, BufferedBodyHandle) {
		let handle = BufferedBodyHandle::default();
		(
			Self {
				inner,
				buffer: BufList::default(),
				trailers: None,
				state: BufferedBodyState::Buffering,
				handle: handle.clone(),
				limit,
				buffered: 0,
			},
			handle,
		)
	}

	pub fn handle(&self) -> BufferedBodyHandle {
		self.handle.clone()
	}
}

impl<B> HttpBody for BufferedBody<B>
where
	B: HttpBody,
	B::Error: Into<axum_core::Error>,
{
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let mut this = self.project();
		loop {
			match *this.state {
				BufferedBodyState::Done => return Poll::Ready(None),
				BufferedBodyState::EmitTrailers => {
					*this.state = BufferedBodyState::Done;
					if let Some(trailers) = this.trailers.take() {
						return Poll::Ready(Some(Ok(Frame::trailers(trailers))));
					}
					return Poll::Ready(None);
				},
				BufferedBodyState::Buffering => {},
			}

			// Keep polling the inner body until EOF. This intentionally withholds all data
			// from the caller so buffered-mode protocols can make one decision over the full body.
			let frame = match futures::ready!(this.inner.as_mut().poll_frame(cx)) {
				Some(Ok(frame)) => frame,
				Some(Err(error)) => {
					*this.state = BufferedBodyState::Done;
					return Poll::Ready(Some(Err(error.into())));
				},
				None => {
					let mut buffer = std::mem::take(this.buffer);
					let len: usize = buffer.remaining();
					let bytes = buffer.copy_to_bytes(len);
					this.handle.complete(bytes.clone());
					// Return all the buffered bytes as one (logical) data frame. If trailers were
					// seen while buffering, replay them on the next poll instead of dropping them.
					*this.state = if this.trailers.is_some() {
						BufferedBodyState::EmitTrailers
					} else {
						BufferedBodyState::Done
					};
					return Poll::Ready(Some(Ok(Frame::data(bytes))));
				},
			};

			match frame.into_data().map_err(Frame::into_trailers) {
				Ok(mut data) => {
					let len = data.remaining();
					let Some(next_buffered) = this.buffered.checked_add(len) else {
						*this.state = BufferedBodyState::Done;
						return Poll::Ready(Some(Err(buffer_limit_exceeded(*this.limit))));
					};
					if next_buffered > *this.limit {
						*this.state = BufferedBodyState::Done;
						return Poll::Ready(Some(Err(buffer_limit_exceeded(*this.limit))));
					}
					let bytes = data.copy_to_bytes(len);
					if bytes.has_remaining() {
						this.buffer.push(bytes);
						*this.buffered = next_buffered;
					}
				},
				Err(Ok(trailers)) => {
					// http_body represents trailers as body frames. Buffering the data must not
					// silently discard those trailers, so keep the last trailer frame for replay.
					*this.trailers = Some(trailers);
				},
				Err(Err(_unknown)) => {
					tracing::warn!("An unknown body frame has been buffered");
					*this.state = BufferedBodyState::Done;
					return Poll::Ready(None);
				},
			}
		}
	}

	fn is_end_stream(&self) -> bool {
		self.state == BufferedBodyState::Done
	}

	fn size_hint(&self) -> SizeHint {
		let mut hint = self.inner.size_hint();
		let buffered = self.buffer.remaining() as u64;
		hint.set_lower(hint.lower().saturating_add(buffered));
		if let Some(upper) = hint.upper() {
			hint.set_upper(upper.saturating_add(buffered));
		}
		hint
	}
}

fn buffer_limit_exceeded(limit: usize) -> axum_core::Error {
	axum_core::Error::new(std::io::Error::other(format!(
		"body exceeded max buffer size of {limit} bytes"
	)))
}
