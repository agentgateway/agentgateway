use ::http::header::CONTENT_TYPE;
use ::http::{HeaderMap, HeaderValue, header};
pub use agent_llm::webhook::{Message, ResponseChoice};
use serde::{Deserialize, Serialize};

use crate::llm::policy::with_default_timeout;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::SimpleBackendReference;
use crate::*;

pub(crate) const REQUEST_PATH: &str = "request";
const RESPONSE_PATH: &str = "response";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailsPromptRequest {
	/// body contains the object which is a list of the Message JSON objects from the prompts in the request
	pub body: PromptMessages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailsPromptResponse {
	/// action is the action to be taken based on the request.
	/// The following actions are available on the response:
	/// - PassAction: No action is required.
	/// - MaskAction: Mask the response body.
	/// - RejectAction: Reject the request.
	pub action: RequestAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailsResponseRequest {
	/// body contains the object with a list of Choice that contains the response content from the LLM.
	pub body: ResponseChoices,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailsResponseResponse {
	/// action is the action to be taken based on the request.
	/// The following actions are available on the response:
	/// - PassAction: No action is required.
	/// - MaskAction: Mask the response body.
	/// - RejectAction: Reject the response.
	pub action: ResponseAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PromptMessages {
	/// List of prompt messages including role and content.
	pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResponseChoices {
	/// list of possible independent responses from the LLM
	pub choices: Vec<ResponseChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PassAction {
	/// reason is a human readable string that explains the reason for the action.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MaskAction {
	/// body contains the modified messages that masked out some of the original contents.
	/// When used in a GuardrailPromptResponse, this should be PromptMessages.
	/// When used in GuardrailResponseResponse, this should be ResponseChoices
	pub body: MaskActionBody,
	/// reason is a human readable string that explains the reason for the action.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RejectAction {
	/// body is the rejection message that will be used for HTTP error response body.
	pub body: String,
	/// status_code is the HTTP status code to be returned in the HTTP error response.
	pub status_code: u16,
	/// reason is a human readable string that explains the reason for the action.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reason: Option<String>,
}

/// Enum for actions available in prompt responses
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum RequestAction {
	Mask(MaskAction),
	Reject(RejectAction),
	Pass(PassAction),
}

/// Enum for actions available in response responses
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum ResponseAction {
	Mask(MaskAction),
	Reject(RejectAction),
	Pass(PassAction),
}

/// Enum for MaskAction body that can be either PromptMessages or ResponseChoices
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaskActionBody {
	PromptMessages(PromptMessages),
	ResponseChoices(ResponseChoices),
}

fn build_request_for_request(
	http_headers: &HeaderMap,
	messages: Vec<Message>,
) -> anyhow::Result<crate::http::Request> {
	let body = GuardrailsPromptRequest {
		body: PromptMessages { messages },
	};
	build_request(&body, REQUEST_PATH, http_headers)
}

fn build_request_for_response(
	http_headers: &HeaderMap,
	choices: Vec<ResponseChoice>,
) -> anyhow::Result<crate::http::Request> {
	let body = GuardrailsResponseRequest {
		body: ResponseChoices { choices },
	};
	build_request(&body, RESPONSE_PATH, http_headers)
}

fn build_request<T: serde::Serialize>(
	body: &T,
	path: &str,
	http_headers: &HeaderMap,
) -> anyhow::Result<crate::http::Request> {
	let body_bytes = serde_json::to_vec(body)?;
	let mut rb = ::http::Request::builder()
		.uri(format!("/{path}"))
		.method(http::Method::POST);
	for (k, v) in http_headers {
		// TODO: this is configurable by users
		if k == header::CONTENT_LENGTH {
			// TODO: probably others
			continue;
		}
		rb = rb.header(k.clone(), v.clone());
	}
	let req = rb
		.header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
		.body(crate::http::Body::from(body_bytes))?;
	Ok(req)
}

pub async fn send_request(
	client: &PolicyClient,
	target: &SimpleBackendReference,
	http_headers: &HeaderMap,
	messages: Vec<Message>,
) -> anyhow::Result<GuardrailsPromptResponse> {
	let whr = with_default_timeout(build_request_for_request(http_headers, messages)?);
	let res = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
		.call_reference(whr, target)
		.await?;
	let parsed = json::from_response_body(res).await?;
	Ok(parsed)
}

pub async fn send_response(
	client: &PolicyClient,
	target: &SimpleBackendReference,
	http_headers: &HeaderMap,
	choices: Vec<ResponseChoice>,
) -> anyhow::Result<GuardrailsResponseResponse> {
	let whr = with_default_timeout(build_request_for_response(http_headers, choices)?);
	let res = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
		.call_reference(whr, target)
		.await?;
	let parsed = json::from_response_body(res).await?;
	Ok(parsed)
}

/// The action a message-processor callout asks the gateway to take. Shared result type across
/// wire formats; a given wire may only produce a subset (the compress wire only ever replaces).
pub enum ProcessorOutcome {
	/// Leave the request unchanged.
	Pass,
	/// Replace the messages with these (raw provider-native objects).
	Replace(Vec<serde_json::Value>),
	/// Reject the request with this body and status.
	Reject { body: String, status_code: u16 },
}

/// Response shape of a message-processor callout — selects how [`call_processor`] parses the
/// reply. The request body is shared across formats (`{messages, model?}`, `model` omitted when
/// absent); only the response differs. Internal — chosen by the call site (guardrail vs
/// compression), never user-configured, because it's fixed by the kind of server being called.
#[derive(Clone, Copy)]
pub enum WireFormat {
	/// Guardrail webhook: `{action:{...}}` response (pass / mask / reject).
	Guardrail,
	/// Flat compress transform, matching Headroom's `POST /v1/compress`: `{messages, ...telemetry}`
	/// response. A pure transform — always replaces.
	Compress,
}

/// Call a message-processor endpoint and parse the response into a [`ProcessorOutcome`]. The
/// transport (backend resolution, buffering, status handling) is shared; `wire` selects the
/// request body and response parser. `messages` are sent verbatim, so callers control fidelity.
#[allow(clippy::too_many_arguments)]
pub async fn call_processor(
	client: &PolicyClient,
	target: &SimpleBackendReference,
	path: &str,
	http_headers: &HeaderMap,
	subtype: OutboundCallSubtype,
	wire: WireFormat,
	messages: &[serde_json::Value],
	model: Option<&str>,
	buffer_limit: Option<crate::transport::BufferLimit>,
) -> anyhow::Result<ProcessorOutcome> {
	// Serialize straight from the borrowed slice: no clone of the (potentially large) message
	// array into an intermediate `Value`, and no owned `model` string.
	#[derive(Serialize)]
	struct CalloutRequest<'a> {
		messages: &'a [serde_json::Value],
		#[serde(skip_serializing_if = "Option::is_none")]
		model: Option<&'a str>,
	}
	let body = CalloutRequest { messages, model };
	// build_request prepends "/", so normalize whether callers pass "request" or "/v1/compress".
	let path = path.trim_start_matches('/');
	let mut req = with_default_timeout(build_request(&body, path, http_headers)?);
	// Large contexts can exceed the default buffer limit; carry the frontend's limit over.
	if let Some(lim) = buffer_limit {
		req.extensions_mut().insert(lim);
	}
	let res = Box::pin(
		client
			.with_outbound(OutboundCallKind::Policy, subtype)
			.call_reference(req, target),
	)
	.await?;

	let status = res.status();
	let lim = http::response_buffer_limit(&res);
	let raw = http::read_body_with_limit(res.into_body(), lim).await?;
	if status != ::http::StatusCode::OK {
		anyhow::bail!("message processor returned status {status}");
	}
	match wire {
		WireFormat::Guardrail => parse_processor_action(&raw),
		WireFormat::Compress => parse_compress_response(&raw),
	}
}

/// Flat compress response (`{messages, ...telemetry}`). Unknown telemetry fields are ignored.
#[derive(Deserialize)]
struct CompressResponse {
	messages: Vec<serde_json::Value>,
	tokens_saved: Option<i64>,
}

/// Parse a flat compress response. Headroom always returns the (possibly rewritten) message
/// array, so this is always a `Replace`.
fn parse_compress_response(raw: &[u8]) -> anyhow::Result<ProcessorOutcome> {
	let resp: CompressResponse =
		serde_json::from_slice(raw).context("invalid compress response")?;
	// Surface the engine's self-reported savings rather than discarding it. This is an estimate
	// from the compressor; the authoritative token counts are recomputed from the returned
	// messages downstream.
	if let Some(saved) = resp.tokens_saved {
		debug!("context compression: engine reported tokens_saved={saved}");
	}
	Ok(ProcessorOutcome::Replace(resp.messages))
}

/// Guardrail `{action: {body, status_code}}` envelope. `body` stays a raw `Value` because its
/// shape is the discriminator: a string means reject, an object with `messages` means mask.
#[derive(Deserialize)]
struct ActionEnvelope {
	action: Option<ActionBody>,
}

#[derive(Deserialize)]
struct ActionBody {
	body: Option<serde_json::Value>,
	status_code: Option<u16>,
}

/// Interpret the `{action: ...}` envelope: reject (string body + status_code) → mask
/// (body.messages) → pass.
fn parse_processor_action(raw: &[u8]) -> anyhow::Result<ProcessorOutcome> {
	let env: ActionEnvelope = serde_json::from_slice(raw).context("invalid guardrail response")?;
	let Some(action) = env.action else {
		return Ok(ProcessorOutcome::Pass);
	};
	let Some(body) = action.body else {
		return Ok(ProcessorOutcome::Pass);
	};
	Ok(match body {
		serde_json::Value::String(text) => ProcessorOutcome::Reject {
			body: text,
			status_code: action
				.status_code
				.unwrap_or_else(|| ::http::StatusCode::FORBIDDEN.as_u16()),
		},
		// Move the array out of the owned body instead of cloning it.
		serde_json::Value::Object(mut map) => match map.get_mut("messages").map(serde_json::Value::take) {
			Some(serde_json::Value::Array(messages)) => ProcessorOutcome::Replace(messages),
			_ => ProcessorOutcome::Pass,
		},
		_ => ProcessorOutcome::Pass,
	})
}

/// Sanity-check raw replacement messages from a processor before applying them. Rejects output
/// that is empty (when the input wasn't), contains non-object messages, or breaks tool-call
/// pairing that was intact in the original request. Shared by context compression and any
/// raw-fidelity webhook that rewrites messages.
pub(crate) fn validate_replacement(
	original: &[serde_json::Value],
	replacement: &[serde_json::Value],
) -> Result<(), String> {
	if replacement.is_empty() && !original.is_empty() {
		return Err("processor returned an empty message array".to_string());
	}
	if replacement.iter().any(|m| !m.is_object()) {
		return Err("processor returned non-object messages".to_string());
	}
	let broken: Vec<_> = pairing_violations(replacement)
		.difference(&pairing_violations(original))
		.cloned()
		.collect();
	if !broken.is_empty() {
		return Err(format!(
			"processor broke tool-call pairing for ids: {}",
			broken.join(", ")
		));
	}
	Ok(())
}

/// Tool-call ids that appear on only one side of the call/result relationship. Understands
/// OpenAI completions (`tool_calls`/`tool_call_id`), Anthropic messages (`tool_use`/
/// `tool_result` content blocks), and OpenAI responses (`function_call`/`function_call_output`
/// items). A model rejects requests where a tool call has no result (or vice versa), so a
/// processor that drops one half of a pair would turn a valid request into a provider error.
fn pairing_violations(messages: &[serde_json::Value]) -> std::collections::BTreeSet<String> {
	let mut calls = std::collections::BTreeSet::new();
	let mut results = std::collections::BTreeSet::new();
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
mod processor_tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn validate_rejects_empty_output_for_nonempty_input() {
		let original = vec![json!({"role": "user", "content": "hi"})];
		assert!(validate_replacement(&original, &[]).is_err());
		assert!(validate_replacement(&[], &[]).is_ok());
	}

	#[test]
	fn validate_rejects_non_object_messages() {
		let original = vec![json!({"role": "user", "content": "hi"})];
		let replacement = vec![json!("not an object")];
		assert!(validate_replacement(&original, &replacement).is_err());
	}

	#[test]
	fn validate_rejects_broken_tool_pairing() {
		let original = vec![
			json!({"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "f", "input": {}}]}),
			json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}),
		];
		let dropped = vec![original[0].clone()];
		assert!(validate_replacement(&original, &dropped).is_err());
		let rewritten = vec![
			original[0].clone(),
			json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "[compressed]"}]}),
		];
		assert!(validate_replacement(&original, &rewritten).is_ok());
	}

	#[test]
	fn validate_allows_preexisting_violations() {
		let original = vec![json!({"role": "assistant", "tool_calls": [{"id": "t9"}]})];
		let replacement = vec![json!({"role": "assistant", "tool_calls": [{"id": "t9"}]})];
		assert!(validate_replacement(&original, &replacement).is_ok());
	}

	#[test]
	fn pairing_covers_openai_completions_and_responses() {
		let paired = vec![
			json!({"role": "assistant", "tool_calls": [{"id": "a", "type": "function"}]}),
			json!({"role": "tool", "tool_call_id": "a", "content": "ok"}),
		];
		assert!(pairing_violations(&paired).is_empty());
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
	fn parse_action_replace_mask_body() {
		let v = json!({"action": {"body": {"messages": [{"role": "user", "content": "x"}]}}});
		assert!(matches!(parse_processor_action(v), ProcessorOutcome::Replace(m) if m.len() == 1));
	}

	#[test]
	fn parse_action_reject_and_pass() {
		let reject = json!({"action": {"body": "no", "status_code": 403}});
		assert!(matches!(
			parse_processor_action(reject),
			ProcessorOutcome::Reject {
				status_code: 403,
				..
			}
		));
		let pass = json!({"action": {"reason": "ok"}});
		assert!(matches!(parse_processor_action(pass), ProcessorOutcome::Pass));
	}

	#[test]
	fn parse_compress_flat_messages() {
		// Headroom returns messages + telemetry at the top level; always a Replace.
		let v = json!({"messages": [{"role": "user", "content": "x"}], "tokens_saved": 42});
		assert!(
			matches!(parse_compress_response(v), Ok(ProcessorOutcome::Replace(m)) if m.len() == 1)
		);
		// Missing messages array is an error (failure handled by the caller).
		assert!(parse_compress_response(json!({"tokens_saved": 0})).is_err());
	}
}
