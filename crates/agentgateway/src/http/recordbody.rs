use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes};
use http_body::{Body, Frame, SizeHint};
use parking_lot::Mutex;

use crate::http::buflist::BufList;

#[derive(Clone, Debug)]
pub struct RecordedBodyHandle {
	state: Arc<Mutex<RecordedBodyState>>,
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
	pub fn bytes(&self) -> Bytes {
		let mut state = self.state.lock();
		match &mut *state {
			RecordedBodyState::Recording(buffer) => {
				// This *should* not happen... but that is a recommended pattern of the caller, not something
				// we enforce.
				let mut buffer = buffer.clone();
				let len = buffer.remaining();
				buffer.copy_to_bytes(len)
			},
			RecordedBodyState::Complete(bytes) => bytes.clone(),
		}
	}

	fn push(&self, bytes: Bytes) {
		let mut state = self.state.lock();
		if let RecordedBodyState::Recording(buffer) = &mut *state {
			buffer.push(bytes);
		} else {
			debug_assert!(false, "push() cannot be called on a complete body handle.")
		}
	}

	fn complete(&self) {
		let mut state = self.state.lock();
		let RecordedBodyState::Recording(buffer) = &mut *state else {
			debug_assert!(
				false,
				"complete() cannot be called on a complete body handle."
			);
			return;
		};
		let mut buffer = std::mem::take(buffer);
		let len = buffer.remaining();
		*state = RecordedBodyState::Complete(buffer.copy_to_bytes(len));
	}
}

/// Wraps an HTTP body and records each polled data frame while passing the
/// bytes through unchanged.
///
/// The recorded data is shared through a [`RecordedBodyHandle`]. This is meant
/// for consumers that inspect the captured bytes after the wrapped body has
/// been fully drained.
#[derive(Debug)]
pub struct RecordedBody<B = crate::http::Body> {
	inner: B,
	handle: RecordedBodyHandle,
}

impl<B> RecordedBody<B> {
	pub fn new(inner: B) -> (Self, RecordedBodyHandle) {
		let handle = RecordedBodyHandle {
			state: Arc::new(Mutex::new(RecordedBodyState::default())),
		};
		(
			Self {
				inner,
				handle: handle.clone(),
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
		let frame = match futures::ready!(Pin::new(&mut this.inner).poll_frame(cx)) {
			Some(Ok(frame)) => frame,
			Some(Err(error)) => return Poll::Ready(Some(Err(error.into()))),
			None => {
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
			Err(Ok(trailers)) => Poll::Ready(Some(Ok(Frame::trailers(trailers)))),
			Err(Err(_unknown)) => {
				tracing::warn!("An unknown body frame has been recorded");
				Poll::Ready(None)
			},
		}
	}

	fn is_end_stream(&self) -> bool {
		self.inner.is_end_stream()
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

	fn mock_body(data: Vec<&'static [u8]>) -> crate::http::Body {
		let iter = data
			.into_iter()
			.map(|d| Ok::<_, crate::http::Error>(Frame::data(Bytes::from_static(d))));
		crate::http::Body::new(StreamBody::new(futures_util::stream::iter(iter)))
	}

	#[tokio::test]
	async fn records_polled_bytes() {
		let (body, recorded) = RecordedBody::new(mock_body(vec![b"hello", b" ", b"world"]));

		assert!(recorded.bytes().is_empty());

		let got = crate::http::Body::new(body)
			.collect()
			.await
			.unwrap()
			.to_bytes();

		assert_eq!(got, Bytes::from_static(b"hello world"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"hello world"));
	}

	#[tokio::test]
	async fn exposes_partial_progress() {
		let (mut body, recorded) = RecordedBody::new(mock_body(vec![b"hello", b"world"]));
		let first = body.frame().await.unwrap().unwrap().into_data().unwrap();

		assert_eq!(first, Bytes::from_static(b"hello"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"hello"));

		let rest = crate::http::Body::new(body)
			.collect()
			.await
			.unwrap()
			.to_bytes();

		assert_eq!(rest, Bytes::from_static(b"world"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"helloworld"));
	}

	#[tokio::test]
	async fn passes_trailers_without_recording_them() {
		let mut trailers = http::HeaderMap::new();
		trailers.insert("x-test", "value".parse().unwrap());
		let frames = vec![
			Ok::<_, crate::http::Error>(Frame::data(Bytes::from_static(b"hello"))),
			Ok::<_, crate::http::Error>(Frame::trailers(trailers.clone())),
		];
		let body = crate::http::Body::new(StreamBody::new(futures_util::stream::iter(frames)));
		let (body, recorded) = RecordedBody::new(body);

		let got = crate::http::Body::new(body).collect().await.unwrap();

		assert_eq!(got.trailers(), Some(&trailers));
		assert_eq!(got.to_bytes(), Bytes::from_static(b"hello"));
		assert_eq!(recorded.bytes(), Bytes::from_static(b"hello"));
	}
}
