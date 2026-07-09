//! Context compression: shrink LLM request messages before they reach the provider.
//!
//! Compression is one use of the shared message-processor callout (see [`super::webhook`]).
//! It sends the request's raw provider-native messages to an external service and applies the
//! messages the service returns. The wire contract is the same `{body:{messages}}` request /
//! `{action}` response envelope the guardrail webhook uses; compression simply runs at a
//! different point in the pipeline (after prompt guards, before token counting), defaults to
//! failing open, and skips small requests.
//!
//! Compression always operates on raw messages — anything less would corrupt tool calls and
//! cache markers — so it has no message-format knob.

use crate::http::Response;
use crate::llm::policy::webhook::ProcessorOutcome;
use crate::llm::policy::{FailureMode, Policy, webhook};
use crate::llm::types::RequestType;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::OutboundCallSubtype;
use crate::types::agent::{HeaderMatch, SimpleBackendReference};
use crate::*;

fn default_failure_mode() -> FailureMode {
	FailureMode::FailOpen
}

pub fn default_min_size_bytes() -> usize {
	// Compression only pays for itself on large contexts; skip the callout for small requests.
	16 * 1024
}

pub fn default_compress_path() -> String {
	"/v1/compress".to_string()
}

/// Context compression shrinks request messages through an external compression service before
/// they reach the LLM, to reduce token spend. Compression is an optimization: by default a
/// failure is ignored and the original request is forwarded unchanged.
#[apply(schema!)]
pub struct ContextCompression {
	/// Backend serving the compression endpoint (a message-processor webhook).
	pub target: SimpleBackendReference,
	/// Request path of the compression endpoint. Defaults to `/v1/compress`.
	#[serde(default = "default_compress_path")]
	pub path: String,
	/// Incoming request headers to forward to the compression service.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub forward_header_matches: Vec<HeaderMatch>,
	/// Behavior when the service is unreachable, errors, or returns unusable messages.
	/// Defaults to `failOpen` (forward the original request unchanged).
	#[serde(default = "default_failure_mode")]
	pub failure_mode: FailureMode,
	/// Minimum serialized size of the message array, in bytes, before compression is attempted.
	/// Requests below the threshold are forwarded untouched. Defaults to 16384.
	#[serde(default = "default_min_size_bytes")]
	pub min_size_bytes: usize,
}

pub enum CompressionOutcome {
	/// Compression not attempted (unsupported format, below size threshold, or engine passed).
	Skipped,
	/// Messages replaced. `original` is the pre-compression snapshot, kept so callers can
	/// revert if the compressed request later fails to render for the provider.
	Applied { original: Vec<serde_json::Value> },
	/// Engine failed and the policy fails open; the request proceeds uncompressed.
	FailedOpen,
	/// The request is rejected (engine failure with failClosed, or an explicit reject action).
	Rejected(Box<Response>),
}

impl ContextCompression {
	pub async fn apply(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		req: &mut dyn RequestType,
		parts: &mut ::http::request::Parts,
	) -> CompressionOutcome {
		// Formats without a message array (embeddings, rerank, detect, ...) are not compressible.
		let Some(original) = req.raw_messages() else {
			debug!("context compression: request format has no message array; skipping");
			return CompressionOutcome::Skipped;
		};

		let size = serialized_size(&original);
		if size < self.min_size_bytes {
			debug!(
				"context compression: messages below size threshold ({size} < {}); skipping",
				self.min_size_bytes
			);
			return CompressionOutcome::Skipped;
		}

		let client = PolicyClient::new(backend_info.inputs.clone());
		let headers = Policy::get_webhook_forward_headers(&parts.headers, &self.forward_header_matches);
		let buffer_limit = parts
			.extensions
			.get::<crate::transport::BufferLimit>()
			.cloned();
		let outcome = webhook::call_processor(
			&client,
			&self.target,
			&self.path,
			&headers,
			OutboundCallSubtype::Compression,
			&original,
			buffer_limit,
		)
		.await;

		match outcome {
			Err(e) => self.fail(&format!("compression call failed: {e}")),
			Ok(ProcessorOutcome::Pass) => CompressionOutcome::Skipped,
			Ok(ProcessorOutcome::Reject { body, status_code }) => {
				CompressionOutcome::Rejected(Box::new(reject_response(&body, status_code)))
			},
			Ok(ProcessorOutcome::Replace(messages)) => {
				if let Err(reason) = webhook::validate_replacement(&original, &messages) {
					return self.fail(&reason);
				}
				if let Err(e) = req.set_raw_messages(messages) {
					// set_raw_messages leaves the request unchanged on error; nothing to revert.
					return self.fail(&format!("compression returned unusable messages: {e}"));
				}
				CompressionOutcome::Applied { original }
			},
		}
	}

	/// Resolve a failure per the configured mode: fail-open proceeds without compression,
	/// fail-closed rejects the request.
	fn fail(&self, reason: &str) -> CompressionOutcome {
		match self.failure_mode {
			FailureMode::FailOpen => {
				warn!("context compression: {reason}; failing open");
				CompressionOutcome::FailedOpen
			},
			FailureMode::FailClosed => {
				warn!("context compression: {reason}; failing closed");
				CompressionOutcome::Rejected(Box::new(reject_response(
					"context compression failed",
					::http::StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
				)))
			},
		}
	}
}

fn reject_response(body: &str, status_code: u16) -> Response {
	::http::response::Builder::new()
		.status(status_code)
		.body(http::Body::from(body.to_owned()))
		.unwrap_or_else(|_| {
			::http::response::Builder::new()
				.status(::http::StatusCode::INTERNAL_SERVER_ERROR)
				.body(http::Body::from("context compression failed"))
				.expect("static response should build")
		})
}

/// Serialized byte size of the message array without allocating the serialized form.
fn serialized_size(messages: &[serde_json::Value]) -> usize {
	struct Counter(usize);
	impl std::io::Write for Counter {
		fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
			self.0 += buf.len();
			Ok(buf.len())
		}
		fn flush(&mut self) -> std::io::Result<()> {
			Ok(())
		}
	}
	let mut counter = Counter(0);
	// Serialization of a Vec<Value> is infallible into an in-memory writer.
	let _ = serde_json::to_writer(&mut counter, messages);
	counter.0
}
