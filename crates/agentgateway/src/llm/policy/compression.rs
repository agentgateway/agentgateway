//! Context compression: shrink LLM request messages before they reach the provider.
//!
//! Compression is one use of the shared message-processor callout (see [`super::webhook`]).
//! It sends the request's raw provider-native messages to an external service and applies the
//! messages the service returns. The on-the-wire shape is Headroom's flat `POST /v1/compress`
//! contract (`{messages, model}` request, `{messages, ...telemetry}` response) —
//! [`webhook::WireFormat::Compress`] — but it shares the guardrail webhook's transport and
//! validation. Compression runs at a different point in the pipeline (after prompt guards,
//! before token counting), defaults to failing open, and skips small requests.
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
	/// Minimum request body size, in bytes, before compression is attempted. Requests below the
	/// threshold are forwarded untouched. Defaults to 16384.
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
		// Gate on the decoded request body size (recorded at parse time) rather than
		// re-serializing the parsed messages. This is the whole body, not just the message
		// array, but it's only a "is there enough to bother compressing" heuristic.
		let Some(size) = parts.extensions.get::<crate::llm::RequestBodyBytes>().map(|b| b.0) else {
			// The body size is recorded during body parsing; its absence means we're on a path
			// that never buffered the body. Skip rather than guess — the safe default.
			debug!("context compression: request body size unrecorded; skipping");
			return CompressionOutcome::Skipped;
		};
		if size < self.min_size_bytes {
			debug!(
				"context compression: request below size threshold ({size} < {}); skipping",
				self.min_size_bytes
			);
			return CompressionOutcome::Skipped;
		}

		// Formats without a message array (embeddings, rerank, detect, ...) are not compressible.
		let Some(original) = req.raw_messages() else {
			debug!("context compression: request format has no message array; skipping");
			return CompressionOutcome::Skipped;
		};

		let model = req.model();
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
			webhook::WireFormat::Compress,
			&original,
			model.as_deref(),
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
				// The compression engine is an upstream dependency; its failure is a bad gateway,
				// not an internal error in agentgateway itself.
				CompressionOutcome::Rejected(Box::new(reject_response(
					"context compression failed",
					::http::StatusCode::BAD_GATEWAY.as_u16(),
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
