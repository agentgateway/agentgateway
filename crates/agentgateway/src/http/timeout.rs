use std::time::Duration;

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

/// Apply the response idle timeout without replacing managed body state.
pub fn apply_response_idle_timeout(
	mut response: crate::http::Response,
	timeout: Duration,
) -> crate::http::Response {
	response.body_mut().set_idle_timeout(timeout);
	response
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
