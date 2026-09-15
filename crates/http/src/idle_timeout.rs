use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body::{Frame, SizeHint};
use tokio::time::Sleep;

use crate::{BodyTimeoutError, RawBody};

/// Bounds pending reads from the original stream, independently of downstream processing.
#[derive(Debug)]
pub(crate) struct IdleTimeout {
	inner: RawBody,
	duration: Duration,
	sleep: Option<Pin<Box<Sleep>>>,
	done: bool,
}

impl IdleTimeout {
	pub(crate) fn new(inner: RawBody, duration: Duration) -> Self {
		Self {
			inner,
			duration,
			sleep: None,
			done: false,
		}
	}
}

impl http_body::Body for IdleTimeout {
	type Data = Bytes;
	type Error = axum_core::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
		let this = self.get_mut();
		if this.done {
			return Poll::Ready(None);
		}
		// Check the upstream first: processing or backpressure may have delayed polling
		// while a frame was already available. Only a pending read starts the window.
		match Pin::new(&mut this.inner).poll_frame(cx) {
			Poll::Ready(frame) => {
				this.sleep = None;
				this.done = matches!(&frame, None | Some(Err(_)));
				Poll::Ready(frame)
			},
			Poll::Pending => {
				let sleep = this
					.sleep
					.get_or_insert_with(|| Box::pin(tokio::time::sleep(this.duration)));
				if sleep.as_mut().poll(cx).is_ready() {
					this.done = true;
					this.sleep = None;
					Poll::Ready(Some(Err(axum_core::Error::new(BodyTimeoutError))))
				} else {
					Poll::Pending
				}
			},
		}
	}

	fn is_end_stream(&self) -> bool {
		self.done || self.inner.is_end_stream()
	}

	fn size_hint(&self) -> SizeHint {
		if self.done {
			SizeHint::with_exact(0)
		} else {
			self.inner.size_hint()
		}
	}
}
