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

use std::sync::LazyLock;

use crate::http::Response;
use crate::llm::policy::webhook::ProcessorOutcome;
use crate::llm::policy::{FailureMode, Policy, webhook};
use crate::llm::types::RequestType;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::{HeaderMatch, HeaderValueMatch, SimpleBackendReference};
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

/// Headers forwarded to the compression engine when `forward_header_matches` is left empty.
///
/// Compression engines (Headroom, litellm, ...) inspect these to decide what is *safe* to
/// compress and how much context they are compressing toward. (Anthropic's in-body
/// `cache_control` markers — the main prompt-cache signal — are not headers; they always
/// survive because raw messages are forwarded verbatim.)
///
/// - `anthropic-version`: API version, i.e. the message schema the engine must parse and emit.
/// - `anthropic-beta` / `openai-beta`: active betas (prompt caching, extended context windows)
///   that change cache behavior and the token budget.
/// - `cache-control`: client cache directives; engines with their own cache layer honor
///   `no-cache`/`no-store`.
///
/// Credentials (`authorization`, `x-api-key`, `cookie`) are deliberately never included, and
/// inbound trace context is not echoed — the gateway's outbound tracing owns that.
///
/// This is a default, not a floor: setting `forward_header_matches` to any non-empty value
/// *replaces* this list entirely (see [`ContextCompression::forward_header_matches`]).
static DEFAULT_FORWARD_HEADERS: LazyLock<Vec<HeaderMatch>> = LazyLock::new(|| {
	// `.*` + get_webhook_forward_headers' full-span check == "forward if present, any value".
	let any = || HeaderValueMatch::Regex(::regex::Regex::new(".*").expect("static regex"));
	[
		"anthropic-version",
		"anthropic-beta",
		"openai-beta",
		"cache-control",
	]
	.into_iter()
	.map(|h| HeaderMatch {
		name: crate::http::HeaderOrPseudo::Header(::http::HeaderName::from_static(h)),
		value: any(),
	})
	.collect()
});

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
	///
	/// When empty, a curated set of non-sensitive cache/context headers is forwarded by default
	/// (`anthropic-version`, `anthropic-beta`, `openai-beta`, `cache-control`), so engines that
	/// decide compressibility from headers behave correctly out of the box. Credentials are never
	/// part of the default. Setting any value here *replaces* that default entirely — it is not
	/// additive, so include the cache headers yourself if you still need them, or compression may
	/// bust prompt caches. To forward nothing, supply a matcher that matches no header.
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
		let Some(size) = parts
			.extensions
			.get::<crate::llm::RequestBodyBytes>()
			.map(|b| b.0)
		else {
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
		// Empty config → curated defaults; any explicit matcher replaces them (not additive).
		let matches = if self.forward_header_matches.is_empty() {
			&*DEFAULT_FORWARD_HEADERS
		} else {
			&self.forward_header_matches
		};
		let headers = Policy::get_webhook_forward_headers(&parts.headers, matches);
		let buffer_limit = parts
			.extensions
			.get::<crate::transport::BufferLimit>()
			.cloned();
		let outcome = webhook::call_processor(
			&client,
			&self.target,
			&self.path,
			&headers,
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
				CompressionOutcome::Rejected(Box::new(reject_response(body, status_code)))
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

fn reject_response(body: impl Into<http::Body>, status_code: u16) -> Response {
	::http::response::Builder::new()
		.status(status_code)
		.body(body.into())
		.unwrap_or_else(|_| {
			::http::response::Builder::new()
				.status(::http::StatusCode::INTERNAL_SERVER_ERROR)
				.body(http::Body::from("context compression failed"))
				.expect("static response should build")
		})
}
