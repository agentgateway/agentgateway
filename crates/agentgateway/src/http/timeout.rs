use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use http_body::{Body, SizeHint};
use pin_project_lite::pin_project;
use tokio::time::{Instant, Sleep, sleep_until};

use crate::*;

#[apply(schema!)]
#[derive(Default, Eq, PartialEq)]
#[cfg_attr(feature = "schema", schemars(rename = "TimeoutPolicy"))]
pub struct Policy {
	/// Maximum time allowed from the start of downstream request processing until response headers
	/// are received. The response body is not included; use `responseIdleTimeout` to bound gaps
	/// between body frames.
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub request_timeout: Option<Duration>,
	/// Maximum time allowed for the upstream backend request.
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub backend_request_timeout: Option<Duration>,
	/// Maximum time the response body may go without producing data.
	///
	/// The window restarts on every body frame, so this bounds the gap between frames rather than
	/// the total time a response may take. It is what terminates a backend that stops producing
	/// data mid-stream without capping how long a legitimately long response may run.
	///
	/// This complements the other two rather than overlapping them: both `requestTimeout` and
	/// `backendRequestTimeout` stop applying once the response headers arrive, so neither places
	/// any bound on how long the response body may take, and neither can distinguish a stalled
	/// stream from a slow one.
	///
	/// The timeout is disabled when this field is unset or set to zero. It does not apply to
	/// responses that switch protocols, so upgraded WebSocket and CONNECT tunnels are never
	/// terminated by it.
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub response_idle_timeout: Option<Duration>,
}

/// Wraps `response`'s body so that it is terminated once it stays idle for `timeout`.
pub fn apply_response_idle_timeout(
	response: crate::http::Response,
	timeout: Duration,
) -> crate::http::Response {
	response.map(|b| crate::http::Body::new(TimeoutBody::new(timeout, b)))
}

pin_project! {
	/// Terminates a body that goes idle for longer than the configured timeout.
	///
	/// The window is armed on the first poll and pushed out again whenever a frame arrives, so it
	/// always measures the gap since the last frame rather than the age of the body.
	pub struct TimeoutBody<B> {
		timeout: Duration,
		#[pin]
		sleep: Option<Sleep>,
		#[pin]
		body: B,
		done: bool,
	}
}

impl<B> TimeoutBody<B> {
	/// Creates a new [`TimeoutBody`].
	pub fn new(timeout: Duration, body: B) -> Self {
		TimeoutBody {
			timeout,
			sleep: None,
			body,
			done: false,
		}
	}
}

impl<B> Body for TimeoutBody<B>
where
	B: Body,
	B::Error: Into<axum_core::BoxError>,
{
	type Data = B::Data;
	type Error = Box<dyn std::error::Error + Send + Sync>;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
		let mut this = self.project();
		if *this.done {
			return Poll::Ready(None);
		}

		// Start the idle window if it is not already running.
		let mut sleep_pinned = if let Some(some) = this.sleep.as_mut().as_pin_mut() {
			some
		} else {
			let deadline = Instant::now() + *this.timeout;
			this.sleep.set(Some(sleep_until(deadline)));
			this.sleep.as_mut().as_pin_mut().unwrap()
		};

		// Error if the body has been idle for the whole window.
		if sleep_pinned.as_mut().poll(cx).is_ready() {
			*this.done = true;
			return Poll::Ready(Some(Err(Box::new(TimeoutError(())))));
		}

		let frame = ready!(this.body.poll_frame(cx));
		if matches!(frame, Some(Ok(_))) {
			// A frame arrived, so the stream is not idle. Push the deadline out rather than dropping
			// the timer and building a new one on the next poll: for token-by-token SSE that would
			// deregister and reregister a timer entry once per token.
			sleep_pinned.reset(Instant::now() + *this.timeout);
		} else {
			*this.done = true;
		}

		Poll::Ready(frame.transpose().map_err(Into::into).transpose())
	}

	fn is_end_stream(&self) -> bool {
		// Once the window has expired `poll_frame` only ever reports the end of the stream, so
		// reporting the inner body's answer here would contradict it.
		self.done || self.body.is_end_stream()
	}

	fn size_hint(&self) -> SizeHint {
		self.body.size_hint()
	}
}

/// Error for [`TimeoutBody`].
#[derive(Debug)]
pub struct TimeoutError(());

impl std::error::Error for TimeoutError {}

impl std::fmt::Display for TimeoutError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "response idle timeout")
	}
}

#[cfg(test)]
mod tests {
	use std::convert::Infallible;

	use bytes::Bytes;
	use futures_util::StreamExt;
	use http_body_util::BodyExt;

	use super::*;

	#[tokio::test(start_paused = true)]
	async fn idle_body_times_out() {
		let pending = futures_util::stream::pending::<Result<Bytes, Infallible>>();
		let mut body = apply_response_idle_timeout(
			crate::http::Response::new(crate::http::Body::from_stream(pending)),
			Duration::from_secs(1),
		)
		.into_body();

		let error = body
			.frame()
			.await
			.expect("timeout should produce a body frame")
			.expect_err("an idle body should time out");

		assert_eq!(error.to_string(), "response idle timeout");
	}

	#[tokio::test(start_paused = true)]
	async fn each_frame_restarts_the_idle_window() {
		// Four frames spaced just inside the window: the body outlives the timeout several times
		// over, but is never idle for a full window.
		let frames = futures_util::stream::iter(0..4).then(|_| async {
			tokio::time::sleep(Duration::from_millis(800)).await;
			Ok::<_, Infallible>(Bytes::from_static(b"data"))
		});
		let body = apply_response_idle_timeout(
			crate::http::Response::new(crate::http::Body::from_stream(frames)),
			Duration::from_secs(1),
		)
		.into_body();

		let collected = body
			.collect()
			.await
			.expect("frames arriving inside the window should not time out");
		assert_eq!(
			collected.to_bytes(),
			Bytes::from_static(b"datadatadatadata")
		);
	}
}
