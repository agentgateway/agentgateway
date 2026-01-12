use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use http_body::{Body, SizeHint};
use pin_project_lite::pin_project;
use tokio::time::{Instant, Sleep, sleep_until};

use crate::*;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Policy {
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub request_timeout: Option<Duration>,
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub backend_request_timeout: Option<Duration>,
}

pub enum BodyTimeout {
	Deadline(Instant),
	None,
}

impl BodyTimeout {
	pub fn apply(self, r: crate::http::Response) -> crate::http::Response {
		r.map(|b| crate::http::Body::new(TimeoutBody::new(self, b)))
	}
}

pin_project! {
	pub struct TimeoutBody<B> {
		timeout: BodyTimeout,
		#[pin]
		sleep: Option<Sleep>,
		#[pin]
		body: B,
	}
}

impl<B> TimeoutBody<B> {
	/// Creates a new [`TimeoutBody`].
	pub fn new(timeout: BodyTimeout, body: B) -> Self {
		TimeoutBody {
			timeout,
			sleep: None,
			body,
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

		// Start the `Sleep` if not active.
		if let BodyTimeout::Deadline(d) = this.timeout {
			// Start the `Sleep` if not active.
			let sleep_pinned = if let Some(some) = this.sleep.as_mut().as_pin_mut() {
				some
			} else {
				this.sleep.set(Some(sleep_until(*d)));
				this.sleep.as_mut().as_pin_mut().unwrap()
			};

			// Error if the timeout has expired.
			if let Poll::Ready(()) = sleep_pinned.poll(cx) {
				return Poll::Ready(Some(Err(Box::new(TimeoutError(())))));
			}
		}

		let frame = ready!(this.body.poll_frame(cx));

		Poll::Ready(frame.transpose().map_err(Into::into).transpose())
	}

	fn is_end_stream(&self) -> bool {
		self.body.is_end_stream()
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
		write!(f, "data was not received within the designated timeout")
	}
}
