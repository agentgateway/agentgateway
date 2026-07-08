//! Context compression: shrink LLM request messages before they reach the provider.
//!
//! The policy is engine-agnostic. The gateway owns the wire contract; any service that
//! implements it (Headroom, a custom summarizer, ...) can be plugged in as an external engine.
//!
//! # External engine wire contract (version 1)
//!
//! `POST <path>` (default `/v1/compress`) with header `x-agw-compression-version: 1` and body:
//!
//! ```json
//! { "messages": [ ... provider-native message objects ... ], "model": "optional-hint" }
//! ```
//!
//! `messages` is the request's native message array, forwarded verbatim so provider-specific
//! blocks (`cache_control`, images, tool calls) survive the round-trip. `model` is a
//! tokenizer/context-window hint, not a routing target. The engine responds `200` with:
//!
//! ```json
//! { "messages": [ ... compressed message objects ... ] }
//! ```
//!
//! Any non-200 status, malformed body, or message array that breaks the request's tool-call
//! pairing is treated as an engine failure and resolved per `failureMode`.

use std::collections::BTreeSet;

use ::http::header::CONTENT_TYPE;
use ::http::{HeaderValue, StatusCode};

use crate::http::Response;
use crate::llm::policy::{FailureMode, with_default_timeout};
use crate::llm::types::RequestType;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::SimpleBackendReference;
use crate::*;

/// Request header that, when set to `true`, skips context compression for that request.
/// The gateway consumes this header; it is not forwarded to the provider.
pub const BYPASS_HEADER: &str = "x-agw-compression-bypass";
/// Wire contract version header sent to external engines.
pub const VERSION_HEADER: &str = "x-agw-compression-version";
const VERSION: &str = "1";

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

/// Context compression shrinks request messages before they reach the LLM to reduce token
/// spend. Compression is an optimization: by default failures are ignored and the original
/// request is forwarded unchanged.
#[apply(schema!)]
pub struct ContextCompression {
	/// Engine that performs the compression.
	pub engine: CompressionEngine,
	/// Behavior when the engine is unreachable, errors, or returns unusable messages.
	/// Defaults to `failOpen` (forward the original request unchanged).
	#[serde(default = "default_failure_mode")]
	pub failure_mode: FailureMode,
	/// Minimum serialized size of the message array, in bytes, before compression is attempted.
	/// Requests below the threshold are forwarded untouched. Defaults to 16384.
	#[serde(default = "default_min_size_bytes")]
	pub min_size_bytes: usize,
}

#[apply(schema!)]
pub enum CompressionEngine {
	/// External compression service implementing the gateway's compression wire contract.
	#[serde(rename = "external")]
	External(ExternalCompressionEngine),
}

/// External service speaking the gateway compression API (see module docs).
#[apply(schema!)]
pub struct ExternalCompressionEngine {
	/// Backend serving the compression endpoint.
	pub target: SimpleBackendReference,
	/// Request path of the compression endpoint. Defaults to `/v1/compress`.
	#[serde(default = "default_compress_path")]
	pub path: String,
}

#[derive(Debug, Serialize)]
struct CompressRequest<'a> {
	messages: &'a [serde_json::Value],
	#[serde(skip_serializing_if = "Option::is_none")]
	model: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
pub struct CompressResponse {
	/// The compressed message array.
	pub messages: Vec<serde_json::Value>,
}

pub enum CompressionOutcome {
	/// Compression not attempted (bypass header, unsupported format, below size threshold).
	Skipped,
	/// Messages replaced. `original` is the pre-compression snapshot, kept so callers can
	/// revert if the compressed request later fails to render for the provider.
	Applied { original: Vec<serde_json::Value> },
	/// Engine failed and the policy fails open; the request proceeds uncompressed.
	FailedOpen,
	/// Engine failed and the policy fails closed; reject with this response.
	Rejected(Box<Response>),
}

impl ContextCompression {
	pub async fn apply(
		&self,
		backend_info: &crate::http::auth::BackendInfo,
		req: &mut dyn RequestType,
		parts: &mut ::http::request::Parts,
	) -> CompressionOutcome {
		// Per-request opt-out. Consume the header so it never reaches the provider.
		let bypass = parts
			.headers
			.remove(BYPASS_HEADER)
			.is_some_and(|v| v.to_str().is_ok_and(|v| v.eq_ignore_ascii_case("true")));
		if bypass {
			debug!("context compression: bypass header set; skipping");
			return CompressionOutcome::Skipped;
		}

		// Formats without a message array (embeddings, rerank, detect, ...) are not compressible.
		let Some(original) = req.raw_messages() else {
			debug!("context compression: request format has no message array; skipping");
			return CompressionOutcome::Skipped;
		};

		let size: usize = original.iter().map(|m| m.to_string().len()).sum::<usize>();
		if size < self.min_size_bytes {
			debug!(
				"context compression: messages below size threshold ({size} < {}); skipping",
				self.min_size_bytes
			);
			return CompressionOutcome::Skipped;
		}

		let model = req.model();
		let client = PolicyClient::new(backend_info.inputs.clone());
		let buffer_limit = parts
			.extensions
			.get::<crate::transport::BufferLimit>()
			.cloned();
		let compressed = match self
			.engine
			.compress(&client, &original, model.as_deref(), buffer_limit)
			.await
		{
			Ok(resp) => resp,
			Err(e) => return self.fail(&format!("engine call failed: {e}")),
		};

		if let Err(reason) = validate_compressed(&original, &compressed.messages) {
			return self.fail(&reason);
		}
		if let Err(e) = req.set_raw_messages(compressed.messages) {
			// set_raw_messages leaves the request unchanged on error; nothing to revert.
			return self.fail(&format!("engine returned unusable messages: {e}"));
		}
		CompressionOutcome::Applied { original }
	}

	/// Resolve an engine failure per the configured mode: fail-open proceeds without
	/// compression, fail-closed rejects the request.
	fn fail(&self, reason: &str) -> CompressionOutcome {
		match self.failure_mode {
			FailureMode::FailOpen => {
				warn!("context compression: {reason}; failing open");
				CompressionOutcome::FailedOpen
			},
			FailureMode::FailClosed => {
				warn!("context compression: {reason}; failing closed");
				CompressionOutcome::Rejected(Box::new(
					::http::response::Builder::new()
						.status(StatusCode::INTERNAL_SERVER_ERROR)
						.body(http::Body::from("context compression failed"))
						.expect("static response should build"),
				))
			},
		}
	}
}

impl CompressionEngine {
	async fn compress(
		&self,
		client: &PolicyClient,
		messages: &[serde_json::Value],
		model: Option<&str>,
		buffer_limit: Option<crate::transport::BufferLimit>,
	) -> anyhow::Result<CompressResponse> {
		match self {
			CompressionEngine::External(e) => e.compress(client, messages, model, buffer_limit).await,
		}
	}
}

impl ExternalCompressionEngine {
	async fn compress(
		&self,
		client: &PolicyClient,
		messages: &[serde_json::Value],
		model: Option<&str>,
		buffer_limit: Option<crate::transport::BufferLimit>,
	) -> anyhow::Result<CompressResponse> {
		let body = serde_json::to_vec(&CompressRequest { messages, model })?;
		let mut req = ::http::Request::builder()
			.uri(&self.path)
			.method(::http::Method::POST)
			.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
			.header(VERSION_HEADER, HeaderValue::from_static(VERSION))
			.body(http::Body::from(body))?;
		// Large contexts can exceed the default buffer limit; carry the frontend's limit over
		// to the engine call so both directions buffer consistently.
		if let Some(lim) = buffer_limit {
			req.extensions_mut().insert(lim);
		}
		let req = with_default_timeout(req);

		let res = Box::pin(
			client
				.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Compression)
				.call_reference(req, &self.target),
		)
		.await?;

		let status = res.status();
		let lim = http::response_buffer_limit(&res);
		let raw = http::read_body_with_limit(res.into_body(), lim).await?;
		if status != StatusCode::OK {
			anyhow::bail!("compression endpoint returned status {status}");
		}
		Ok(serde_json::from_slice(&raw)?)
	}
}

/// Sanity-check an engine's output before applying it. Rejects output that is empty (when the
/// input wasn't), contains non-object messages, or breaks tool-call pairing that was intact in
/// the original request.
fn validate_compressed(
	original: &[serde_json::Value],
	compressed: &[serde_json::Value],
) -> Result<(), String> {
	if compressed.is_empty() && !original.is_empty() {
		return Err("engine returned an empty message array".to_string());
	}
	if compressed.iter().any(|m| !m.is_object()) {
		return Err("engine returned non-object messages".to_string());
	}
	let broken: Vec<_> = pairing_violations(compressed)
		.difference(&pairing_violations(original))
		.cloned()
		.collect();
	if !broken.is_empty() {
		return Err(format!(
			"engine broke tool-call pairing for ids: {}",
			broken.join(", ")
		));
	}
	Ok(())
}

/// Tool-call ids that appear on only one side of the call/result relationship. Understands
/// OpenAI completions (`tool_calls`/`tool_call_id`), Anthropic messages (`tool_use`/
/// `tool_result` content blocks), and OpenAI responses (`function_call`/`function_call_output`
/// items). A model rejects requests where a tool call has no result (or vice versa), so a
/// compressor that drops one half of a pair would turn a valid request into a provider error.
fn pairing_violations(messages: &[serde_json::Value]) -> BTreeSet<String> {
	let mut calls = BTreeSet::new();
	let mut results = BTreeSet::new();
	let as_str = |v: &serde_json::Value, k: &str| {
		v.get(k)
			.and_then(|v| v.as_str())
			.map(std::string::ToString::to_string)
	};
	for m in messages {
		// OpenAI completions
		if let Some(tool_calls) = m.get("tool_calls").and_then(|v| v.as_array()) {
			calls.extend(tool_calls.iter().filter_map(|c| as_str(c, "id")));
		}
		if let Some(id) = as_str(m, "tool_call_id") {
			results.insert(id);
		}
		// Anthropic messages content blocks
		if let Some(parts) = m.get("content").and_then(|v| v.as_array()) {
			for p in parts {
				match p.get("type").and_then(|v| v.as_str()) {
					Some("tool_use") => calls.extend(as_str(p, "id")),
					Some("tool_result") => results.extend(as_str(p, "tool_use_id")),
					_ => {},
				}
			}
		}
		// OpenAI responses items
		match m.get("type").and_then(|v| v.as_str()) {
			Some("function_call") => calls.extend(as_str(m, "call_id")),
			Some("function_call_output") => results.extend(as_str(m, "call_id")),
			_ => {},
		}
	}
	calls.symmetric_difference(&results).cloned().collect()
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn validate_rejects_empty_output_for_nonempty_input() {
		let original = vec![json!({"role": "user", "content": "hi"})];
		assert!(validate_compressed(&original, &[]).is_err());
		assert!(validate_compressed(&[], &[]).is_ok());
	}

	#[test]
	fn validate_rejects_non_object_messages() {
		let original = vec![json!({"role": "user", "content": "hi"})];
		let compressed = vec![json!("not an object")];
		assert!(validate_compressed(&original, &compressed).is_err());
	}

	#[test]
	fn validate_rejects_broken_tool_pairing() {
		// Anthropic-style: tool_use in assistant content paired with tool_result in user content.
		let original = vec![
			json!({"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "f", "input": {}}]}),
			json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}),
		];
		// Compressor dropped the result half of the pair.
		let compressed = vec![original[0].clone()];
		assert!(validate_compressed(&original, &compressed).is_err());
		// Keeping the pair intact is fine, even with rewritten content.
		let rewritten = vec![
			original[0].clone(),
			json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "[compressed]"}]}),
		];
		assert!(validate_compressed(&original, &rewritten).is_ok());
	}

	#[test]
	fn validate_allows_preexisting_violations() {
		// The original request was already unpaired; the compressor shouldn't be blamed for it.
		let original = vec![json!({"role": "assistant", "tool_calls": [{"id": "t9"}]})];
		let compressed = vec![json!({"role": "assistant", "tool_calls": [{"id": "t9"}]})];
		assert!(validate_compressed(&original, &compressed).is_ok());
	}

	#[test]
	fn pairing_covers_openai_completions_and_responses() {
		// completions: tool_calls + role=tool message
		let paired = vec![
			json!({"role": "assistant", "tool_calls": [{"id": "a", "type": "function"}]}),
			json!({"role": "tool", "tool_call_id": "a", "content": "ok"}),
		];
		assert!(pairing_violations(&paired).is_empty());
		// responses: function_call + function_call_output items
		let paired = vec![
			json!({"type": "function_call", "call_id": "b", "name": "f", "arguments": "{}"}),
			json!({"type": "function_call_output", "call_id": "b", "output": "ok"}),
		];
		assert!(pairing_violations(&paired).is_empty());
		let unpaired =
			vec![json!({"type": "function_call", "call_id": "c", "name": "f", "arguments": "{}"})];
		assert_eq!(pairing_violations(&unpaired).len(), 1);
	}

	#[test]
	fn compress_response_rejects_non_array_messages() {
		let body = serde_json::json!({ "messages": "oops not an array" });
		assert!(serde_json::from_value::<CompressResponse>(body).is_err());
	}
}
