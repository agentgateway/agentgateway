use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes};
use http_body::{Body, Frame, SizeHint};
use parking_lot::Mutex;

use crate::{Body as ManagedBody, BufList};

/// Shared view of bytes captured while a body is consumed.
///
/// Recording is passive: enabling it does not read the body. The managed body
/// feeds it as frames are polled, so its bytes may be
/// incomplete until consumption finishes. It is primarily used for logging
/// after a body has been sent. In contrast, [`crate::BodyInspection`] is the
/// immediate result of actively inspecting a body before forwarding. A managed
/// [`ManagedBody`] retains this observer across content replacement and resets
/// it to capture the replacement.
#[derive(Clone, Debug)]
pub struct RecordedBodyHandle {
	inner: Arc<Mutex<RecordedBodyHandleInner>>,
	limit: usize,
}

#[derive(Debug)]
struct RecordedBodyHandleInner {
	state: RecordedBodyState,
	recorded: usize,
	exceeded_limit: bool,
}

#[derive(Debug)]
enum RecordedBodyState {
	Recording(BufList),
	Complete(Bytes),
}

impl Default for RecordedBodyState {
	fn default() -> Self {
		Self::Recording(BufList::default())
	}
}

impl RecordedBodyHandle {
	pub(crate) fn limit(&self) -> usize {
		self.limit
	}

	pub(crate) fn new(limit: usize) -> Self {
		Self {
			inner: Arc::new(Mutex::new(RecordedBodyHandleInner {
				state: RecordedBodyState::default(),
				recorded: 0,
				exceeded_limit: false,
			})),
			limit,
		}
	}

	/// Returns whether recording observed more bytes than the configured limit.
	pub fn exceeded_limit(&self) -> bool {
		self.inner.lock().exceeded_limit
	}

	/// Whether recording reached the end of the body, rather than just a prefix.
	pub fn is_complete(&self) -> bool {
		matches!(self.inner.lock().state, RecordedBodyState::Complete(_))
	}

	pub fn bytes(&self) -> Bytes {
		let mut inner = self.inner.lock();
		match &mut inner.state {
			RecordedBodyState::Recording(buffer) => {
				// Logging interrupted delivery and debug tracing can request a prefix
				// before completion. Leave the recording intact for subsequent frames.
				let mut buffer = buffer.clone();
				let len = buffer.remaining();
				buffer.copy_to_bytes(len)
			},
			RecordedBodyState::Complete(bytes) => bytes.clone(),
		}
	}

	pub(crate) fn push(&self, bytes: Bytes) {
		let mut inner = self.inner.lock();
		let remaining = self.limit.saturating_sub(inner.recorded);
		if bytes.len() > remaining {
			inner.exceeded_limit = true;
		}
		if let RecordedBodyState::Recording(buffer) = &mut inner.state {
			let to_record = bytes.len().min(remaining);
			if to_record == 0 {
				return;
			}
			buffer.push(bytes.slice(0..to_record));
			inner.recorded += to_record;
		} else {
			debug_assert!(false, "push() cannot be called on a complete body handle.")
		}
	}

	pub(crate) fn complete(&self) {
		let mut inner = self.inner.lock();
		let RecordedBodyState::Recording(buffer) = &mut inner.state else {
			return;
		};
		let mut buffer = std::mem::take(buffer);
		let len = buffer.remaining();
		inner.state = RecordedBodyState::Complete(buffer.copy_to_bytes(len));
	}

	pub(crate) fn reset(&self) {
		let mut inner = self.inner.lock();
		inner.state = RecordedBodyState::default();
		inner.recorded = 0;
		inner.exceeded_limit = false;
	}
}

/// Wraps an HTTP body and records each polled data frame while passing the
/// bytes through unchanged.
///
/// The recorded data is shared through a [`RecordedBodyHandle`]. This is meant
/// for consumers that inspect the captured bytes after the wrapped body has
/// been fully drained.
#[derive(Debug)]
pub struct RecordedBody<B = ManagedBody> {
	inner: B,
	handle: RecordedBodyHandle,
	finished: bool,
}

impl<B> RecordedBody<B> {
	pub fn new(inner: B) -> (Self, RecordedBodyHandle) {
		Self::new_with_limit(inner, usize::MAX)
	}

	pub fn new_with_limit(inner: B, limit: usize) -> (Self, RecordedBodyHandle) {
		let handle = RecordedBodyHandle::new(limit);
		(
			Self {
				inner,
				handle: handle.clone(),
				finished: false,
			},
			handle,
		)
	}

	pub fn handle(&self) -> RecordedBodyHandle {
		self.handle.clone()
	}
}

impl<B> Body for RecordedBody<B>
where
	B: Body + Unpin,
	B::Error: Into<axum_core::Error>,
{
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let this = self.get_mut();
		if this.finished {
			return Poll::Ready(None);
		}
		let frame = match std::task::ready!(Pin::new(&mut this.inner).poll_frame(cx)) {
			Some(Ok(frame)) => frame,
			Some(Err(error)) => {
				this.finished = true;
				return Poll::Ready(Some(Err(error.into())));
			},
			None => {
				this.finished = true;
				this.handle.complete();
				return Poll::Ready(None);
			},
		};

		match frame.into_data().map_err(Frame::into_trailers) {
			Ok(mut data) => {
				let len = data.remaining();
				let bytes = data.copy_to_bytes(len);
				if bytes.has_remaining() {
					this.handle.push(bytes.clone());
				}
				Poll::Ready(Some(Ok(Frame::data(bytes))))
			},
			Err(Ok(trailers)) => {
				this.finished = true;
				this.handle.complete();
				Poll::Ready(Some(Ok(Frame::trailers(trailers))))
			},
			Err(Err(_unknown)) => {
				tracing::warn!("An unknown body frame has been recorded");
				this.finished = true;
				Poll::Ready(None)
			},
		}
	}

	fn is_end_stream(&self) -> bool {
		self.finished || self.inner.is_end_stream()
	}

	fn size_hint(&self) -> SizeHint {
		self.inner.size_hint()
	}
}

#[cfg(test)]
mod tests {
	use bytes::Bytes;
	use http_body::Frame;
	use http_body_util::{BodyExt, StreamBody};

	use super::*;

	fn mock_body(data: Vec<&'static [u8]>) -> crate::Body {
		let iter = data
			.into_iter()
			.map(|d| Ok::<_, crate::Error>(Frame::data(Bytes::from_static(d))));
		crate::Body::new(StreamBody::new(futures_util::stream::iter(iter)))
	}

	#[tokio::test]
	async fn records_polled_bytes() {
		let (body, recorded) = RecordedBody::new(mock_body(vec![b"hello", b" ", b"world"]));

		assert!(recorded.bytes().is_empty());

		let got = crate::Body::new(body).collect().await.unwrap().to_bytes();

		assert_eq!(got, Bytes::from_static(b"hello world"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"hello world"));
	}

	#[tokio::test]
	async fn does_not_repoll_inner_after_eof() {
		let inner = crate::Body::new(http_body_util::StreamBody::new(
			futures_util::stream::unfold(false, |emitted| async move {
				if emitted {
					None
				} else {
					Some((
						Ok::<_, crate::Error>(Frame::data(Bytes::from_static(b"body"))),
						true,
					))
				}
			}),
		));
		let (mut body, _handle) = RecordedBody::new(inner);

		assert!(body.frame().await.is_some(), "data frame should be present");
		assert!(body.frame().await.is_none(), "stream should report EOF");
		assert!(body.frame().await.is_none(), "stream must remain at EOF");
	}

	#[tokio::test]
	async fn reuses_completed_bytes() {
		let (body, recorded) = RecordedBody::new(mock_body(vec![b"hello", b"world"]));

		let got = crate::Body::new(body).collect().await.unwrap().to_bytes();

		assert_eq!(got, Bytes::from_static(b"helloworld"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"helloworld"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"helloworld"));
	}

	#[tokio::test]
	async fn records_up_to_limit() {
		let (body, recorded) = RecordedBody::new_with_limit(mock_body(vec![b"hello", b"world"]), 7);

		let got = crate::Body::new(body).collect().await.unwrap().to_bytes();

		assert_eq!(got, Bytes::from_static(b"helloworld"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"hellowo"));
		assert!(recorded.exceeded_limit());
	}

	#[tokio::test]
	async fn zero_limit_records_nothing() {
		let (body, recorded) = RecordedBody::new_with_limit(mock_body(vec![b"hello"]), 0);

		let got = crate::Body::new(body).collect().await.unwrap().to_bytes();

		assert_eq!(got, Bytes::from_static(b"hello"));
		assert!(recorded.bytes().is_empty());
		assert!(recorded.exceeded_limit());
	}

	#[tokio::test]
	async fn exposes_partial_progress() {
		let (mut body, recorded) = RecordedBody::new(mock_body(vec![b"hello", b"world"]));
		let first = body.frame().await.unwrap().unwrap().into_data().unwrap();

		assert_eq!(first, Bytes::from_static(b"hello"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"hello"));

		let rest = crate::Body::new(body).collect().await.unwrap().to_bytes();

		assert_eq!(rest, Bytes::from_static(b"world"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"helloworld"));
	}

	#[tokio::test]
	async fn passes_trailers_without_recording_them() {
		let mut trailers = http::HeaderMap::new();
		trailers.insert("x-test", "value".parse().unwrap());
		let frames = vec![
			Ok::<_, crate::Error>(Frame::data(Bytes::from_static(b"hello"))),
			Ok::<_, crate::Error>(Frame::trailers(trailers.clone())),
		];
		let body = crate::Body::new(StreamBody::new(futures_util::stream::iter(frames)));
		let (body, recorded) = RecordedBody::new(body);

		let got = crate::Body::new(body).collect().await.unwrap();

		assert_eq!(got.trailers(), Some(&trailers));
		assert_eq!(got.to_bytes(), Bytes::from_static(b"hello"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"hello"));
	}
}
