use std::collections::{HashMap, HashSet};
use std::time::Instant;

use agent_core::strng;
use axum_core::body::Body;
use base64::Engine;
use bytes::Bytes;
use itertools::Itertools;

use crate::parse::sse::StrictSseJsonEvent;
use crate::types::ResponseType;
use crate::types::messages::typed as messages;
use crate::types::responses::typed as responses;
use crate::{AIError, StreamingUsageGuard, parse, types};

#[derive(Debug, Clone, Default)]
pub struct State {
	tools: HashMap<String, DeclaredTool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WrappedKind {
	NamespaceFunction,
	NamespaceCustom,
	Custom,
	LocalShell,
	Shell,
	ApplyPatch,
}

impl WrappedKind {
	// The wrapper-object field holding the underlying tool's payload, shared by every site that
	// builds or validates a wrapped tool's schema so the two can't drift apart.
	fn field_name(&self) -> &'static str {
		match self {
			WrappedKind::NamespaceFunction => "arguments",
			WrappedKind::NamespaceCustom | WrappedKind::Custom => "input",
			WrappedKind::LocalShell | WrappedKind::Shell => "action",
			WrappedKind::ApplyPatch => "operation",
		}
	}

	// The item-id prefix used both for a completed wrapped tool call and for the streamed
	// "added" item that precedes it, so the two stay in sync.
	fn id_prefix(&self) -> &'static str {
		match self {
			WrappedKind::NamespaceFunction => "fc",
			WrappedKind::NamespaceCustom | WrappedKind::Custom => "ctc",
			WrappedKind::LocalShell => "lsc",
			WrappedKind::Shell => "shc",
			WrappedKind::ApplyPatch => "apc",
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeclaredTool {
	Function,
	Wrapped(WrappedTool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WrappedTool {
	kind: WrappedKind,
	name: String,
	namespace: Option<String>,
}

pub fn translate(req: &types::responses::Request) -> Result<(Vec<u8>, State), AIError> {
	let raw = serde_json::to_value(req).map_err(AIError::RequestMarshal)?;
	let output_format = responses_output_format(&raw)?;
	let mut state = validate_top_level(&raw)?;
	if req
		.temperature
		.is_some_and(|temperature| !temperature.is_finite() || !(0.0..=1.0).contains(&temperature))
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses temperature must be between 0 and 1"
		)));
	}
	let tools = translate_tools(&raw, &mut state)?;
	let tool_choice = translate_tool_choice(&raw, &state)?;
	let raw_input = raw.get("input").ok_or_else(|| {
		AIError::UnsupportedConversion(strng::literal!("Responses input is required"))
	})?;
	let instructions = raw.get("instructions").and_then(serde_json::Value::as_str);
	let (messages, system) = translate_input(raw_input, instructions, &state)?;
	let max_tokens = req
		.max_output_tokens
		.map(usize::try_from)
		.transpose()
		.map_err(|_| {
			AIError::UnsupportedConversion(strng::literal!("Responses max_output_tokens is too large"))
		})?
		.unwrap_or(4096);
	let output_config = output_format.map(|format| messages::OutputConfig {
		effort: None,
		format: Some(format),
	});
	let user_id = raw
		.get("safety_identifier")
		.and_then(serde_json::Value::as_str)
		.filter(|value| !value.is_empty())
		.or_else(|| {
			raw
				.get("user")
				.and_then(serde_json::Value::as_str)
				.filter(|value| !value.is_empty())
		});
	let metadata = user_id.map(|user_id| messages::Metadata {
		fields: HashMap::from([("user_id".to_string(), user_id.to_string())]),
	});
	let translated = messages::Request {
		messages,
		system,
		model: req.model.clone().unwrap_or_default(),
		max_tokens,
		stop_sequences: Vec::new(),
		stream: req.stream.unwrap_or(false),
		temperature: req.temperature,
		top_p: req.top_p,
		top_k: None,
		tools: (!tools.is_empty()).then_some(tools),
		tool_choice,
		metadata,
		thinking: None,
		output_config,
	};
	let body = serde_json::to_vec(&translated).map_err(AIError::RequestMarshal)?;
	Ok((body, state))
}

pub fn translate_error(_bytes: &Bytes, status: ::http::StatusCode) -> Result<Bytes, AIError> {
	let error_type = match status {
		::http::StatusCode::BAD_REQUEST => "invalid_request_error",
		::http::StatusCode::UNAUTHORIZED => "authentication_error",
		::http::StatusCode::FORBIDDEN => "permission_error",
		::http::StatusCode::NOT_FOUND => "not_found_error",
		::http::StatusCode::CONFLICT => "conflict_error",
		::http::StatusCode::PAYLOAD_TOO_LARGE => "request_too_large",
		::http::StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
		_ => "server_error",
	};
	let body = serde_json::json!({
		"error": {
			"message": format!(
				"Upstream Anthropic request failed with HTTP {}",
				status.as_u16()
			),
			"type": error_type,
			"param": null,
			"code": null,
		}
	});
	Ok(Bytes::from(
		serde_json::to_vec(&body).map_err(AIError::ResponseMarshal)?,
	))
}

struct TranslatedResponse {
	response: types::responses::Response,
	input_tokens: u64,
	cache_creation_input_tokens: Option<u64>,
	reasoning_tokens: Option<u64>,
	provider_model: String,
}

impl ResponseType for TranslatedResponse {
	fn to_llm_response(&self, include_completion_in_log: bool) -> crate::LLMResponse {
		let mut result = self.response.to_llm_response(include_completion_in_log);
		result.input_tokens = Some(self.input_tokens);
		result.total_tokens = result
			.output_tokens
			.and_then(|output| self.input_tokens.checked_add(output));
		result.cache_creation_input_tokens = self.cache_creation_input_tokens;
		result.reasoning_tokens = self.reasoning_tokens;
		result.provider_model = Some(strng::new(&self.provider_model));
		if result.service_tier.as_deref() == Some("default") {
			result.service_tier = Some(strng::literal!("standard"));
		}
		result
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(&self.response)
	}

	fn to_webhook_choices(&self) -> Vec<crate::webhook::ResponseChoice> {
		self.response.to_webhook_choices()
	}

	fn set_webhook_choices(
		&mut self,
		choices: Vec<crate::webhook::ResponseChoice>,
	) -> anyhow::Result<()> {
		self.response.set_webhook_choices(choices)
	}
}

pub fn translate_response(
	bytes: &Bytes,
	model: &str,
	state: &State,
	buffer_limit: usize,
) -> Result<Box<dyn ResponseType>, AIError> {
	let raw: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid_response())?;
	validate_response(&raw, state)?;
	let response: messages::MessagesResponse =
		serde_json::from_value(raw).map_err(|_| invalid_response())?;
	let cache_creation_input_tokens = response
		.usage
		.cache_creation_input_tokens
		.map(|tokens| tokens as u64);
	let input_tokens = u64::try_from(response.usage.input_tokens).map_err(|_| invalid_response())?;
	let provider_model = response.model.clone();
	let stop_reason = response.stop_reason.ok_or_else(invalid_response)?;
	let refusal = matches!(stop_reason, messages::StopReason::Refusal);
	let (status, incomplete_reason) = terminal_status(stop_reason).ok_or_else(invalid_response)?;
	let usage = responses_usage(&response.usage)?;
	let reasoning_tokens = response
		.usage
		.output_tokens_details
		.as_ref()
		.and_then(|details| details.thinking_tokens)
		.map(u64::try_from)
		.transpose()
		.map_err(|_| invalid_response())?;
	let service_tier = response
		.usage
		.service_tier
		.as_deref()
		.map(public_service_tier)
		.transpose()?;
	let output = response_output(
		&response.id,
		response.content,
		status,
		refusal,
		state,
		buffer_limit,
	)?;
	let mut value = serde_json::json!({
		"id": format!("resp_{}", response.id),
		"object": "response",
		"created_at": chrono::Utc::now().timestamp() as u64,
		"status": status,
		"output": output,
		"model": model,
		"usage": usage,
	});
	if let Some(reason) = incomplete_reason {
		value["incomplete_details"] = serde_json::json!({"reason": reason});
	}
	if let Some(tier) = service_tier {
		value["service_tier"] = serde_json::json!(tier);
	}
	let response = serde_json::from_value(value).map_err(|_| invalid_response())?;
	// The per-item checks in response_output measure the compact Value this module builds, but
	// some vendored async-openai output types (e.g. LocalShellExecAction's optional fields) have
	// no skip_serializing_if, so an absent optional field round-trips back out as an explicit
	// `null` once deserialized into the typed Response. Check what will actually be serialized,
	// not just what was measured on the way in.
	let final_bytes = serde_json::to_vec(&response)
		.map_err(|_| invalid_response())?
		.len();
	if final_bytes > buffer_limit {
		return Err(response_output_too_large());
	}
	Ok(Box::new(TranslatedResponse {
		response,
		input_tokens,
		cache_creation_input_tokens,
		reasoning_tokens,
		provider_model,
	}))
}

fn validate_response(raw: &serde_json::Value, state: &State) -> Result<(), AIError> {
	let object = raw.as_object().ok_or_else(invalid_response)?;
	if has_unknown_field(
		object,
		&[
			"id",
			"type",
			"role",
			"content",
			"model",
			"stop_reason",
			"stop_sequence",
			"usage",
			"copilot_usage",
			"stop_details",
		],
	) || object.get("type").and_then(serde_json::Value::as_str) != Some("message")
		|| object.get("role").and_then(serde_json::Value::as_str) != Some("assistant")
		|| !required_response_string(object, "id")
		|| !required_response_string(object, "model")
	{
		return Err(invalid_response());
	}
	let stop_reason = object
		.get("stop_reason")
		.and_then(serde_json::Value::as_str)
		.ok_or_else(invalid_response)?;
	match (stop_reason, object.get("stop_sequence")) {
		("stop_sequence", Some(serde_json::Value::String(sequence))) if !sequence.is_empty() => {},
		("stop_sequence", _) => return Err(invalid_response()),
		(_, None | Some(serde_json::Value::Null)) => {},
		_ => return Err(invalid_response()),
	}
	let usage = object
		.get("usage")
		.and_then(serde_json::Value::as_object)
		.ok_or_else(invalid_response)?;
	if has_unknown_field(
		usage,
		&[
			"input_tokens",
			"output_tokens",
			"cache_creation_input_tokens",
			"cache_read_input_tokens",
			"service_tier",
			"cache_creation",
			"inference_geo",
			"output_tokens_details",
		],
	) || !usage.contains_key("input_tokens")
		|| !usage.contains_key("output_tokens")
	{
		return Err(invalid_response());
	}
	if let Some(details) = usage.get("output_tokens_details") {
		let details = details.as_object().ok_or_else(invalid_response)?;
		if has_unknown_field(details, &["thinking_tokens"])
			|| details
				.get("thinking_tokens")
				.is_some_and(|tokens| tokens.as_u64().is_none())
		{
			return Err(invalid_response());
		}
	}
	let content = object
		.get("content")
		.and_then(serde_json::Value::as_array)
		.ok_or_else(invalid_response)?;
	let mut tool_ids = HashSet::new();
	let mut has_tool = false;
	for block in content {
		let block = block.as_object().ok_or_else(invalid_response)?;
		match block.get("type").and_then(serde_json::Value::as_str) {
			Some("text") => {
				if has_unknown_field(block, &["type", "text"])
					|| !matches!(block.get("text"), Some(serde_json::Value::String(_)))
				{
					return Err(invalid_response());
				}
			},
			Some("thinking" | "redacted_thinking") => return Err(invalid_response()),
			Some("tool_use") => {
				has_tool = true;
				validate_response_tool(block, state, &mut tool_ids)?;
			},
			_ => return Err(invalid_response()),
		}
	}
	if (stop_reason == "tool_use" && !has_tool)
		|| (has_tool && matches!(stop_reason, "end_turn" | "stop_sequence" | "refusal"))
	{
		return Err(invalid_response());
	}
	Ok(())
}

fn validate_response_tool(
	block: &serde_json::Map<String, serde_json::Value>,
	state: &State,
	tool_ids: &mut HashSet<String>,
) -> Result<(), AIError> {
	if has_unknown_field(block, &["type", "id", "name", "input", "caller"])
		|| !required_response_string(block, "id")
		|| !required_response_string(block, "name")
		|| !direct_tool_caller(block.get("caller"))
	{
		return Err(invalid_response());
	}
	let id = block["id"].as_str().expect("tool id validated");
	if !tool_ids.insert(id.to_string()) {
		return Err(invalid_response());
	}
	let name = block["name"].as_str().expect("tool name validated");
	let input = block
		.get("input")
		.and_then(serde_json::Value::as_object)
		.ok_or_else(invalid_response)?;
	match state.tools.get(name).ok_or_else(invalid_response)? {
		DeclaredTool::Function => {},
		DeclaredTool::Wrapped(wrapped) => validate_wrapped_response_input(wrapped, input)?,
	}
	Ok(())
}

fn direct_tool_caller(caller: Option<&serde_json::Value>) -> bool {
	match caller {
		None => true,
		Some(serde_json::Value::Object(caller)) => {
			caller.len() == 1 && caller.get("type").and_then(serde_json::Value::as_str) == Some("direct")
		},
		Some(_) => false,
	}
}

fn validate_wrapped_response_input(
	wrapped: &WrappedTool,
	input: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), AIError> {
	let field = wrapped.kind.field_name();
	if has_unknown_field(input, &[field]) || !input.contains_key(field) {
		return Err(invalid_response());
	}
	match wrapped.kind {
		WrappedKind::NamespaceFunction => input[field]
			.is_object()
			.then_some(())
			.ok_or_else(invalid_response),
		WrappedKind::NamespaceCustom | WrappedKind::Custom => input[field]
			.is_string()
			.then_some(())
			.ok_or_else(invalid_response),
		WrappedKind::LocalShell | WrappedKind::Shell | WrappedKind::ApplyPatch => {
			validate_action(&wrapped.kind, &input[field]).map_err(|_| invalid_response())
		},
	}
}

fn required_response_string(
	object: &serde_json::Map<String, serde_json::Value>,
	field: &str,
) -> bool {
	object
		.get(field)
		.and_then(serde_json::Value::as_str)
		.is_some_and(|value| !value.is_empty())
}

fn terminal_status(
	stop_reason: messages::StopReason,
) -> Option<(&'static str, Option<&'static str>)> {
	match stop_reason {
		messages::StopReason::EndTurn
		| messages::StopReason::StopSequence
		| messages::StopReason::ToolUse
		| messages::StopReason::Refusal => Some(("completed", None)),
		messages::StopReason::MaxTokens | messages::StopReason::ModelContextWindowExceeded => {
			Some(("incomplete", Some("max_output_tokens")))
		},
		messages::StopReason::PauseTurn => None,
	}
}

fn responses_usage(usage: &messages::Usage) -> Result<responses::ResponseUsage, AIError> {
	response_usage(
		usage.input_tokens,
		usage.output_tokens,
		usage.cache_read_input_tokens.unwrap_or_default(),
		usage.cache_creation_input_tokens.unwrap_or_default(),
		usage
			.output_tokens_details
			.as_ref()
			.and_then(|details| details.thinking_tokens),
	)
	.map_err(|_| invalid_response())
}

fn response_usage(
	input: usize,
	output: usize,
	cache_read: usize,
	cache_creation: usize,
	thinking: Option<usize>,
) -> Result<responses::ResponseUsage, ()> {
	let input_tokens = input
		.checked_add(cache_read)
		.and_then(|tokens| tokens.checked_add(cache_creation))
		.and_then(|tokens| u32::try_from(tokens).ok())
		.ok_or(())?;
	let output_tokens = u32::try_from(output).map_err(|_| ())?;
	let reasoning_tokens = thinking
		.map(u32::try_from)
		.transpose()
		.map_err(|_| ())?
		.unwrap_or_default();
	if reasoning_tokens > output_tokens {
		return Err(());
	}
	Ok(responses::ResponseUsage {
		input_tokens,
		output_tokens,
		total_tokens: input_tokens.checked_add(output_tokens).ok_or(())?,
		input_tokens_details: responses::InputTokenDetails {
			cached_tokens: u32::try_from(cache_read).map_err(|_| ())?,
		},
		output_tokens_details: responses::OutputTokenDetails { reasoning_tokens },
	})
}

fn public_service_tier(service_tier: &str) -> Result<responses::ServiceTier, AIError> {
	match service_tier {
		"standard" => Ok(responses::ServiceTier::Default),
		"priority" => Ok(responses::ServiceTier::Priority),
		_ => Err(invalid_response()),
	}
}

fn response_output(
	message_id: &str,
	content: Vec<messages::ContentBlock>,
	status: &str,
	refusal: bool,
	state: &State,
	buffer_limit: usize,
) -> Result<Vec<serde_json::Value>, AIError> {
	let mut output = Vec::new();
	let mut pending_text: Option<(usize, Vec<serde_json::Value>)> = None;
	let mut retained_bytes = 0usize;
	for (index, block) in content.into_iter().enumerate() {
		if let messages::ContentBlock::Text(text) = block {
			let (_, parts) = pending_text.get_or_insert_with(|| (index, Vec::new()));
			parts.push(if refusal {
				serde_json::json!({"type":"refusal","refusal":text.text})
			} else {
				serde_json::json!({
					"type": "output_text",
					"annotations": [],
					"logprobs": null,
					"text": text.text,
				})
			});
			continue;
		}
		flush_response_text(
			&mut output,
			&mut pending_text,
			message_id,
			status,
			buffer_limit,
			&mut retained_bytes,
		)?;
		match block {
			messages::ContentBlock::Thinking { .. } | messages::ContentBlock::RedactedThinking { .. } => {
				return Err(invalid_response());
			},
			messages::ContentBlock::ToolUse {
				id, name, input, ..
			} => {
				let item = response_tool_output(message_id, index, status, &id, &name, input, state)?;
				retain_response_item(&mut output, item, buffer_limit, &mut retained_bytes)?;
			},
			_ => return Err(invalid_response()),
		}
	}
	flush_response_text(
		&mut output,
		&mut pending_text,
		message_id,
		status,
		buffer_limit,
		&mut retained_bytes,
	)?;
	if refusal && output.is_empty() {
		let item = serde_json::json!({
			"type": "message",
			"id": format!("msg_{message_id}_0"),
			"role": "assistant",
			"phase": "final_answer",
			"status": status,
			"content": [{"type": "refusal", "refusal": ""}],
		});
		retain_response_item(&mut output, item, buffer_limit, &mut retained_bytes)?;
	}
	Ok(output)
}

fn retain_response_item(
	output: &mut Vec<serde_json::Value>,
	item: serde_json::Value,
	buffer_limit: usize,
	retained_bytes: &mut usize,
) -> Result<(), AIError> {
	let bytes = serde_json::to_vec(&item)
		.map_err(|_| invalid_response())?
		.len();
	let total = retained_bytes
		.checked_add(bytes)
		.filter(|total| *total <= buffer_limit)
		.ok_or_else(response_output_too_large)?;
	*retained_bytes = total;
	output.push(item);
	Ok(())
}

fn response_output_too_large() -> AIError {
	AIError::InvalidResponse(strng::literal!(
		"Anthropic Messages response output exceeds the configured size limit"
	))
}

fn flush_response_text(
	output: &mut Vec<serde_json::Value>,
	pending: &mut Option<(usize, Vec<serde_json::Value>)>,
	message_id: &str,
	status: &str,
	buffer_limit: usize,
	retained_bytes: &mut usize,
) -> Result<(), AIError> {
	if let Some((index, content)) = pending.take() {
		let item = serde_json::json!({
			"type": "message",
			"id": format!("msg_{message_id}_{index}"),
			"role": "assistant",
			"phase": "final_answer",
			"status": status,
			"content": content,
		});
		retain_response_item(output, item, buffer_limit, retained_bytes)?;
	}
	Ok(())
}

fn response_tool_output(
	message_id: &str,
	index: usize,
	status: &str,
	call_id: &str,
	upstream_name: &str,
	input: serde_json::Value,
	state: &State,
) -> Result<serde_json::Value, AIError> {
	let declared = state
		.tools
		.get(upstream_name)
		.ok_or_else(invalid_response)?;
	match declared {
		DeclaredTool::Function => Ok(serde_json::json!({
			"type": "function_call",
			"id": format!("fc_{message_id}_{index}"),
			"call_id": call_id,
			"name": upstream_name,
			"arguments": serde_json::to_string(&input).map_err(|_| invalid_response())?,
			"status": status,
		})),
		DeclaredTool::Wrapped(wrapped) => {
			wrapped_response_tool(message_id, index, status, call_id, wrapped, &input)
		},
	}
}

fn wrapped_response_tool(
	message_id: &str,
	index: usize,
	status: &str,
	call_id: &str,
	wrapped: &WrappedTool,
	input: &serde_json::Value,
) -> Result<serde_json::Value, AIError> {
	let id = format!("{}_{message_id}_{index}", wrapped.kind.id_prefix());
	match wrapped.kind {
		WrappedKind::NamespaceFunction => Ok(serde_json::json!({
			"type": "function_call",
			"id": id,
			"call_id": call_id,
			"namespace": wrapped.namespace,
			"name": wrapped.name,
			"arguments": serde_json::to_string(&input["arguments"]).map_err(|_| invalid_response())?,
			"status": status,
		})),
		WrappedKind::NamespaceCustom | WrappedKind::Custom => {
			let mut item = serde_json::json!({
				"type": "custom_tool_call",
				"id": id,
				"call_id": call_id,
				"name": wrapped.name,
				"input": input["input"],
			});
			if let Some(namespace) = &wrapped.namespace {
				item["namespace"] = serde_json::Value::String(namespace.clone());
			}
			Ok(item)
		},
		WrappedKind::LocalShell => Ok(serde_json::json!({
			"type": "local_shell_call",
			"id": id,
			"call_id": call_id,
			"action": input["action"],
			"status": status,
		})),
		WrappedKind::Shell => Ok(serde_json::json!({
			"type": "shell_call",
			"id": id,
			"call_id": call_id,
			"action": input["action"],
			"status": status,
			"environment": {"type": "local"},
		})),
		WrappedKind::ApplyPatch => Ok(serde_json::json!({
			"type": "apply_patch_call",
			"id": id,
			"call_id": call_id,
			"operation": input["operation"],
			"status": "completed",
		})),
	}
}

struct StreamTextBlock {
	index: usize,
	output_index: u32,
	content_index: u32,
	item_id: String,
	text: String,
}

struct StreamToolBlock {
	index: usize,
	output_index: u32,
	item_id: String,
	call_id: String,
	upstream_name: String,
	json: String,
}

enum StreamBlock {
	Text(StreamTextBlock),
	Tool(StreamToolBlock),
}

#[derive(Default)]
struct ResponsesStreamState {
	sequence_number: u64,
	message_id: Option<String>,
	upstream_model: Option<String>,
	initial_usage: Option<messages::Usage>,
	terminal_usage: Option<messages::MessageDeltaUsage>,
	stop_reason: Option<messages::StopReason>,
	stop_sequence: Option<String>,
	active_block: Option<StreamBlock>,
	output: Vec<responses::OutputItem>,
	retained_output_bytes: usize,
	next_block_index: usize,
	tool_ids: HashSet<String>,
	tool_id_bytes: usize,
	saw_tool: bool,
	saw_message_delta: bool,
	terminated: bool,
	terminal_ready: bool,
	first_visible_at: Option<Instant>,
	completion: Option<String>,
	late_refusal: bool,
}

impl ResponsesStreamState {
	fn sequence(&mut self) -> Result<u64, ()> {
		let current = self.sequence_number;
		self.sequence_number = self.sequence_number.checked_add(1).ok_or(())?;
		Ok(current)
	}

	fn error_event(&mut self) -> Vec<(&'static str, responses::ResponseStreamEvent)> {
		if self.terminated {
			return Vec::new();
		}
		self.terminated = true;
		let sequence_number = self.sequence().unwrap_or(u64::MAX);
		let (code, message) = if self.late_refusal {
			(
				"refusal_after_streaming",
				"Anthropic reported a refusal after content had already been streamed as ordinary \
				 text; the emitted content cannot be retyped as a refusal"
					.to_string(),
			)
		} else {
			(
				"server_error",
				"Upstream Anthropic stream was invalid".to_string(),
			)
		};
		vec![(
			"error",
			responses::ResponseStreamEvent::ResponseError(responses::ResponseErrorEvent {
				sequence_number,
				code: Some(code.to_string()),
				message,
				param: None,
			}),
		)]
	}

	fn retain_output(&mut self, item: responses::OutputItem) -> Result<(), ()> {
		let bytes = serde_json::to_vec(&item).map_err(|_| ())?.len();
		self.retained_output_bytes = self
			.retained_output_bytes
			.checked_add(bytes)
			.and_then(|total| total.checked_add(usize::from(!self.output.is_empty())))
			.ok_or(())?;
		self.output.push(item);
		Ok(())
	}

	fn retain_text_part(&mut self, block: &StreamTextBlock) -> Result<(), ()> {
		let output_index = usize::try_from(block.output_index).map_err(|_| ())?;
		if output_index == self.output.len() {
			if block.content_index != 0 {
				return Err(());
			}
			let mut item = stream_message_item(
				block.item_id.clone(),
				block.text.clone(),
				responses::OutputStatus::Completed,
			);
			set_output_item_status(&mut item, responses::OutputStatus::InProgress)?;
			return self.retain_output(item);
		}
		if output_index + 1 != self.output.len() {
			return Err(());
		}
		let responses::OutputItem::Message(message) = &mut self.output[output_index] else {
			return Err(());
		};
		if message.id != block.item_id
			|| message.status != responses::OutputStatus::InProgress
			|| message.content.len() != usize::try_from(block.content_index).map_err(|_| ())?
		{
			return Err(());
		}
		let part = responses::OutputMessageContent::OutputText(responses::OutputTextContent {
			annotations: Vec::new(),
			logprobs: None,
			text: block.text.clone(),
		});
		let added_bytes = serde_json::to_vec(&part)
			.map_err(|_| ())?
			.len()
			.checked_add(usize::from(!message.content.is_empty()))
			.ok_or(())?;
		self.retained_output_bytes = self
			.retained_output_bytes
			.checked_add(added_bytes)
			.ok_or(())?;
		message.content.push(part);
		Ok(())
	}

	fn retain_tool_id(&mut self, id: String) -> Result<(), ()> {
		let tool_id_bytes = self.tool_id_bytes.checked_add(id.len()).ok_or(())?;
		if !self.tool_ids.insert(id) {
			return Err(());
		}
		self.tool_id_bytes = tool_id_bytes;
		Ok(())
	}

	fn ensure_retained_limit(&self, limit: usize, downstream_model: &str) -> Result<(), ()> {
		fn add(total: &mut usize, bytes: usize) -> Result<(), ()> {
			*total = total.checked_add(bytes).ok_or(())?;
			Ok(())
		}

		let mut total = downstream_model.len();
		for value in [
			self.message_id.as_deref(),
			self.upstream_model.as_deref(),
			self.stop_sequence.as_deref(),
			self.completion.as_deref(),
			self
				.initial_usage
				.as_ref()
				.and_then(|usage| usage.service_tier.as_deref()),
		]
		.into_iter()
		.flatten()
		{
			add(&mut total, value.len())?;
		}
		if self.message_id.is_some() {
			add(
				&mut total,
				"resp_".len() + self.message_id.as_deref().map_or(0, str::len) + downstream_model.len(),
			)?;
		}
		add(&mut total, self.tool_id_bytes)?;
		if let Some(block) = &self.active_block {
			match block {
				StreamBlock::Text(block) => {
					add(&mut total, block.item_id.len())?;
					add(&mut total, block.text.len())?;
				},
				StreamBlock::Tool(block) => {
					for value in [
						&block.item_id,
						&block.call_id,
						&block.upstream_name,
						&block.json,
					] {
						add(&mut total, value.len())?;
					}
				},
			}
		}
		add(&mut total, self.retained_output_bytes)?;
		add(&mut total, 2)?;
		(total <= limit).then_some(()).ok_or(())
	}

	fn mark_visible(&mut self) {
		self.first_visible_at.get_or_insert_with(Instant::now);
	}
}

fn stream_output_part(text: String) -> responses::OutputContent {
	responses::OutputContent::OutputText(responses::OutputTextContent {
		annotations: Vec::new(),
		logprobs: None,
		text,
	})
}

fn stream_refusal_part(refusal: String) -> responses::OutputContent {
	responses::OutputContent::Refusal(responses::RefusalContent { refusal })
}

fn stream_message_item(
	item_id: String,
	text: String,
	status: responses::OutputStatus,
) -> responses::OutputItem {
	let in_progress = matches!(status, responses::OutputStatus::InProgress);
	responses::OutputItem::Message(responses::OutputMessage {
		content: if text.is_empty() && in_progress {
			Vec::new()
		} else {
			vec![responses::OutputMessageContent::OutputText(
				responses::OutputTextContent {
					annotations: Vec::new(),
					logprobs: None,
					text,
				},
			)]
		},
		id: item_id,
		role: responses::AssistantRole::Assistant,
		phase: Some(responses::MessagePhase::FinalAnswer),
		status,
	})
}

fn stream_empty_refusal_item(
	item_id: String,
	status: responses::OutputStatus,
) -> responses::OutputItem {
	responses::OutputItem::Message(responses::OutputMessage {
		content: vec![responses::OutputMessageContent::Refusal(
			responses::RefusalContent {
				refusal: String::new(),
			},
		)],
		id: item_id,
		role: responses::AssistantRole::Assistant,
		phase: Some(responses::MessagePhase::FinalAnswer),
		status,
	})
}

fn stream_item(value: serde_json::Value) -> Result<responses::OutputItem, ()> {
	serde_json::from_value(value).map_err(|_| ())
}

fn set_output_item_status(
	item: &mut responses::OutputItem,
	status: responses::OutputStatus,
) -> Result<(), ()> {
	match item {
		responses::OutputItem::Message(message) => message.status = status,
		responses::OutputItem::FunctionCall(call) => call.status = Some(status),
		responses::OutputItem::Reasoning(reasoning) => reasoning.status = Some(status),
		responses::OutputItem::LocalShellCall(call) => call.status = status,
		responses::OutputItem::ShellCall(call) => {
			call.status = match status {
				responses::OutputStatus::InProgress => responses::FunctionShellCallStatus::InProgress,
				responses::OutputStatus::Completed => responses::FunctionShellCallStatus::Completed,
				responses::OutputStatus::Incomplete => responses::FunctionShellCallStatus::Incomplete,
			};
		},
		responses::OutputItem::ApplyPatchCall(_) | responses::OutputItem::CustomToolCall(_) => {},
		_ => return Err(()),
	}
	Ok(())
}

fn stream_tool_added_item(
	message_id: &str,
	index: usize,
	call_id: &str,
	upstream_name: &str,
	state: &State,
) -> Result<(String, Option<responses::OutputItem>), ()> {
	let declared = state.tools.get(upstream_name).ok_or(())?;
	let prefix = match declared {
		DeclaredTool::Function => "fc",
		DeclaredTool::Wrapped(wrapped) => wrapped.kind.id_prefix(),
	};
	let item_id = format!("{prefix}_{message_id}_{index}");
	let added = match declared {
		DeclaredTool::Function => stream_item(serde_json::json!({
			"type":"function_call", "id":item_id.clone(), "call_id":call_id,
			"name":upstream_name, "arguments":"", "status":"in_progress"
		}))
		.map(Some),
		DeclaredTool::Wrapped(wrapped) => match wrapped.kind {
			WrappedKind::NamespaceFunction => stream_item(serde_json::json!({
				"type":"function_call", "id":item_id.clone(), "call_id":call_id,
				"namespace":wrapped.namespace, "name":wrapped.name,
				"arguments":"", "status":"in_progress"
			}))
			.map(Some),
			WrappedKind::NamespaceCustom | WrappedKind::Custom => stream_item(serde_json::json!({
				"type":"custom_tool_call", "id":item_id.clone(), "call_id":call_id,
				"namespace":wrapped.namespace, "name":wrapped.name, "input":""
			}))
			.map(Some),
			WrappedKind::LocalShell | WrappedKind::Shell | WrappedKind::ApplyPatch => Ok(None),
		},
	};
	Ok((item_id, added?))
}

fn stream_usage(
	initial: &messages::Usage,
	terminal: &messages::MessageDeltaUsage,
) -> Result<responses::ResponseUsage, ()> {
	let thinking = stream_thinking_tokens(initial, terminal)?;
	if terminal
		.input_tokens
		.is_some_and(|value| value < initial.input_tokens)
		|| terminal
			.output_tokens
			.is_some_and(|value| value < initial.output_tokens)
		|| terminal
			.cache_read_input_tokens
			.is_some_and(|value| value < initial.cache_read_input_tokens.unwrap_or_default())
		|| terminal
			.cache_creation_input_tokens
			.is_some_and(|value| value < initial.cache_creation_input_tokens.unwrap_or_default())
	{
		return Err(());
	}
	let input = terminal.input_tokens.unwrap_or(initial.input_tokens);
	let output = terminal.output_tokens.unwrap_or(initial.output_tokens);
	let cache_read = terminal
		.cache_read_input_tokens
		.or(initial.cache_read_input_tokens)
		.unwrap_or_default();
	let cache_creation = terminal
		.cache_creation_input_tokens
		.or(initial.cache_creation_input_tokens)
		.unwrap_or_default();
	response_usage(input, output, cache_read, cache_creation, thinking)
}

fn stream_service_tier(tier: Option<&str>) -> Result<Option<responses::ServiceTier>, ()> {
	tier.map(public_service_tier).transpose().map_err(|_| ())
}

fn strict_thinking_tokens(
	details: Option<&messages::OutputTokensDetails>,
) -> Result<Option<usize>, ()> {
	let Some(details) = details else {
		return Ok(None);
	};
	match &details.rest {
		serde_json::Value::Null => {},
		serde_json::Value::Object(fields) if fields.is_empty() => {},
		_ => return Err(()),
	}
	Ok(details.thinking_tokens)
}

fn stream_thinking_tokens(
	initial: &messages::Usage,
	terminal: &messages::MessageDeltaUsage,
) -> Result<Option<usize>, ()> {
	let initial = strict_thinking_tokens(initial.output_tokens_details.as_ref())?;
	let terminal = strict_thinking_tokens(terminal.output_tokens_details.as_ref())?;
	if terminal
		.zip(initial)
		.is_some_and(|(terminal, initial)| terminal < initial)
	{
		return Err(());
	}
	Ok(terminal.or(initial))
}

fn commit_stream_telemetry(
	stream: &ResponsesStreamState,
	log: &StreamingUsageGuard,
) -> Result<(), ()> {
	let initial = stream.initial_usage.as_ref().ok_or(())?;
	let terminal = stream.terminal_usage.as_ref().ok_or(())?;
	let usage = stream_usage(initial, terminal)?;
	let input_tokens =
		u64::try_from(terminal.input_tokens.unwrap_or(initial.input_tokens)).map_err(|_| ())?;
	let output_tokens = u64::from(usage.output_tokens);
	let total_tokens = input_tokens.checked_add(output_tokens).ok_or(())?;
	let cache_creation = terminal
		.cache_creation_input_tokens
		.or(initial.cache_creation_input_tokens)
		.map(|value| value as u64);
	let reasoning_tokens = stream_thinking_tokens(initial, terminal)?
		.map(u64::try_from)
		.transpose()
		.map_err(|_| ())?;
	let provider_model = stream.upstream_model.clone();
	let completion = stream.completion.clone();
	let first_token = stream.first_visible_at;
	let service_tier = initial.service_tier.clone();
	log.update(|info| {
		info.response.input_tokens = Some(input_tokens);
		info.response.output_tokens = Some(output_tokens);
		info.response.total_tokens = Some(total_tokens);
		info.response.reasoning_tokens = reasoning_tokens;
		info.response.cached_input_tokens = Some(u64::from(usage.input_tokens_details.cached_tokens));
		info.response.cache_creation_input_tokens = cache_creation;
		info.response.provider_model = provider_model.as_deref().map(strng::new);
		info.response.first_token = first_token;
		info.response.service_tier = service_tier.as_deref().map(strng::new);
		if let Some(completion) = completion.clone() {
			info.response.completion = Some(vec![completion]);
		}
	});
	Ok(())
}

pub fn translate_stream(
	body: Body,
	buffer_limit: usize,
	log: StreamingUsageGuard,
	model: &str,
	include_completion_in_log: bool,
	conversion_state: State,
) -> Body {
	let mut stream = ResponsesStreamState {
		completion: include_completion_in_log.then(String::new),
		..Default::default()
	};
	let mut response_builder: Option<types::responses::ResponseBuilder> = None;
	let model = model.to_string();

	parse::sse::json_transform_strict_with_eof::<
		messages::MessagesStreamEvent,
		responses::ResponseStreamEvent,
	>(body, buffer_limit, move |event| {
		if stream.terminated {
			return (Vec::new(), true);
		}
		let StrictSseJsonEvent::Data { event_name, data } = event else {
			let output = match event {
				StrictSseJsonEvent::Eof if stream.terminated => Vec::new(),
				StrictSseJsonEvent::Eof | StrictSseJsonEvent::Done | StrictSseJsonEvent::TransportError => {
					stream.error_event()
				},
				StrictSseJsonEvent::Data { .. } => unreachable!(),
			};
			return (output, stream.terminated);
		};
		let Ok(event) = data else {
			let output = stream.error_event();
			return (output, true);
		};
		if event_name.as_deref() != Some(event.event_name()) {
			let output = stream.error_event();
			return (output, true);
		}

		let sequence_checkpoint = stream.sequence_number;
		let result = (|| -> Result<Vec<(&'static str, responses::ResponseStreamEvent)>, ()> {
			match event {
				messages::MessagesStreamEvent::MessageStart { message } => {
					if stream.message_id.is_some()
						|| message.r#type != "message"
						|| message.role != messages::Role::Assistant
						|| !message.content.is_empty()
						|| message.stop_reason.is_some()
						|| message.stop_sequence.is_some()
						|| message.id.is_empty()
						|| message.model.is_empty()
					{
						return Err(());
					}
					let service_tier = stream_service_tier(message.usage.service_tier.as_deref())?;
					let builder =
						types::responses::ResponseBuilder::new(format!("resp_{}", message.id), model.clone());
					let mut snapshot = builder.response(responses::Status::InProgress, None, None, None);
					snapshot.service_tier = service_tier;
					let created =
						responses::ResponseStreamEvent::ResponseCreated(responses::ResponseCreatedEvent {
							sequence_number: stream.sequence()?,
							response: snapshot.clone(),
						});
					let in_progress = responses::ResponseStreamEvent::ResponseInProgress(
						responses::ResponseInProgressEvent {
							sequence_number: stream.sequence()?,
							response: snapshot,
						},
					);
					stream.message_id = Some(message.id);
					stream.upstream_model = Some(message.model);
					stream.initial_usage = Some(message.usage);
					response_builder = Some(builder);
					Ok(vec![
						("response.created", created),
						("response.in_progress", in_progress),
					])
				},
				messages::MessagesStreamEvent::ContentBlockStart {
					index,
					content_block,
				} => {
					if stream.message_id.is_none()
						|| stream.active_block.is_some()
						|| stream.saw_message_delta
						|| index != stream.next_block_index
					{
						return Err(());
					}
					let output_index = u32::try_from(stream.output.len()).map_err(|_| ())?;
					let message_id = stream.message_id.clone().ok_or(())?;
					stream.next_block_index = stream.next_block_index.checked_add(1).ok_or(())?;
					match content_block {
						messages::ContentBlock::Text(text) => {
							if !text.text.is_empty() || text.citations.is_some() || text.cache_control.is_some() {
								return Err(());
							}
							let (output_index, content_index, item_id, add_item) =
								if let Some(responses::OutputItem::Message(message)) = stream.output.last() {
									(
										u32::try_from(stream.output.len() - 1).map_err(|_| ())?,
										u32::try_from(message.content.len()).map_err(|_| ())?,
										message.id.clone(),
										false,
									)
								} else {
									(output_index, 0, format!("msg_{message_id}_{index}"), true)
								};
							stream.active_block = Some(StreamBlock::Text(StreamTextBlock {
								index,
								output_index,
								content_index,
								item_id: item_id.clone(),
								text: String::new(),
							}));
							let mut events = Vec::new();
							if add_item {
								events.push((
									"response.output_item.added",
									responses::ResponseStreamEvent::ResponseOutputItemAdded(
										responses::ResponseOutputItemAddedEvent {
											sequence_number: stream.sequence()?,
											output_index,
											item: stream_message_item(
												item_id.clone(),
												String::new(),
												responses::OutputStatus::InProgress,
											),
										},
									),
								));
							}
							events.push((
								"response.content_part.added",
								responses::ResponseStreamEvent::ResponseContentPartAdded(
									responses::ResponseContentPartAddedEvent {
										sequence_number: stream.sequence()?,
										item_id,
										output_index,
										content_index,
										part: stream_output_part(String::new()),
									},
								),
							));
							Ok(events)
						},
						messages::ContentBlock::ToolUse {
							id,
							name,
							input,
							caller,
							cache_control,
						} => {
							if id.is_empty()
								|| name.is_empty()
								|| input.as_object().is_none_or(|input| !input.is_empty())
								|| !direct_tool_caller(caller.as_ref())
								|| cache_control.is_some()
							{
								return Err(());
							}
							stream.retain_tool_id(id.clone())?;
							stream.saw_tool = true;
							let (item_id, added) =
								stream_tool_added_item(&message_id, index, &id, &name, &conversion_state)?;
							stream.active_block = Some(StreamBlock::Tool(StreamToolBlock {
								index,
								output_index,
								item_id,
								call_id: id,
								upstream_name: name,
								json: String::new(),
							}));
							if let Some(item) = added {
								Ok(vec![(
									"response.output_item.added",
									responses::ResponseStreamEvent::ResponseOutputItemAdded(
										responses::ResponseOutputItemAddedEvent {
											sequence_number: stream.sequence()?,
											output_index,
											item,
										},
									),
								)])
							} else {
								Ok(Vec::new())
							}
						},
						messages::ContentBlock::Thinking { .. }
						| messages::ContentBlock::RedactedThinking { .. } => Err(()),
						_ => Err(()),
					}
				},
				messages::MessagesStreamEvent::ContentBlockDelta { index, delta } => {
					if stream.saw_message_delta {
						return Err(());
					}
					let mut block = stream.active_block.take().ok_or(())?;
					let events = match (&mut block, delta) {
						(StreamBlock::Text(block), messages::ContentBlockDelta::TextDelta { text })
							if block.index == index =>
						{
							block.text.push_str(&text);
							if let Some(completion) = stream.completion.as_mut() {
								completion.push_str(&text);
							}
							if !text.is_empty() {
								stream.mark_visible();
							}
							vec![(
								"response.output_text.delta",
								responses::ResponseStreamEvent::ResponseOutputTextDelta(
									responses::ResponseTextDeltaEvent {
										sequence_number: stream.sequence()?,
										item_id: block.item_id.clone(),
										output_index: block.output_index,
										content_index: block.content_index,
										delta: text,
										logprobs: None,
									},
								),
							)]
						},
						(
							StreamBlock::Tool(block),
							messages::ContentBlockDelta::InputJsonDelta { partial_json },
						) if block.index == index => {
							block.json.push_str(&partial_json);
							if !partial_json.is_empty() {
								stream.mark_visible();
							}
							let declared = conversion_state.tools.get(&block.upstream_name).ok_or(())?;
							match declared {
								DeclaredTool::Function => {
									vec![(
										"response.function_call_arguments.delta",
										responses::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
											responses::ResponseFunctionCallArgumentsDeltaEvent {
												sequence_number: stream.sequence()?,
												item_id: block.item_id.clone(),
												output_index: block.output_index,
												delta: partial_json,
											},
										),
									)]
								},
								DeclaredTool::Wrapped(_) => Vec::new(),
							}
						},
						_ => return Err(()),
					};
					stream.active_block = Some(block);
					Ok(events)
				},
				messages::MessagesStreamEvent::ContentBlockStop { index } => {
					let block = stream.active_block.take().ok_or(())?;
					match block {
						StreamBlock::Text(block) if block.index == index => {
							stream.retain_text_part(&block)?;
							Ok(vec![
								(
									"response.output_text.done",
									responses::ResponseStreamEvent::ResponseOutputTextDone(
										responses::ResponseTextDoneEvent {
											sequence_number: stream.sequence()?,
											item_id: block.item_id.clone(),
											output_index: block.output_index,
											content_index: block.content_index,
											text: block.text.clone(),
											logprobs: None,
										},
									),
								),
								(
									"response.content_part.done",
									responses::ResponseStreamEvent::ResponseContentPartDone(
										responses::ResponseContentPartDoneEvent {
											sequence_number: stream.sequence()?,
											item_id: block.item_id,
											output_index: block.output_index,
											content_index: block.content_index,
											part: stream_output_part(block.text),
										},
									),
								),
							])
						},
						StreamBlock::Tool(block) if block.index == index => {
							let raw = if block.json.is_empty() {
								"{}"
							} else {
								&block.json
							};
							let input: serde_json::Value = serde_json::from_str(raw).map_err(|_| ())?;
							let input_object = input.as_object().ok_or(())?;
							let declared = conversion_state.tools.get(&block.upstream_name).ok_or(())?;
							if let DeclaredTool::Wrapped(wrapped) = declared {
								validate_wrapped_response_input(wrapped, input_object).map_err(|_| ())?;
							}
							let item = stream_item(
								response_tool_output(
									stream.message_id.as_deref().ok_or(())?,
									index,
									"in_progress",
									&block.call_id,
									&block.upstream_name,
									input.clone(),
									&conversion_state,
								)
								.map_err(|_| ())?,
							)?;
							stream.retain_output(item.clone())?;
							match declared {
								DeclaredTool::Function
								| DeclaredTool::Wrapped(WrappedTool {
									kind: WrappedKind::NamespaceFunction,
									..
								}) => {
									let arguments = match declared {
										DeclaredTool::Function => raw.to_string(),
										DeclaredTool::Wrapped(_) => {
											serde_json::to_string(&input_object["arguments"]).map_err(|_| ())?
										},
									};
									let mut events = Vec::new();
									if matches!(declared, DeclaredTool::Wrapped(_)) {
										stream.mark_visible();
										events.push((
											"response.function_call_arguments.delta",
											responses::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
												responses::ResponseFunctionCallArgumentsDeltaEvent {
													sequence_number: stream.sequence()?,
													item_id: block.item_id.clone(),
													output_index: block.output_index,
													delta: arguments.clone(),
												},
											),
										));
									}
									events.push((
										"response.function_call_arguments.done",
										responses::ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
											responses::ResponseFunctionCallArgumentsDoneEvent {
												name: Some(match declared {
													DeclaredTool::Function => block.upstream_name.clone(),
													DeclaredTool::Wrapped(wrapped) => wrapped.name.clone(),
												}),
												sequence_number: stream.sequence()?,
												item_id: block.item_id.clone(),
												output_index: block.output_index,
												arguments,
											},
										),
									));
									Ok(events)
								},
								DeclaredTool::Wrapped(WrappedTool {
									kind: WrappedKind::NamespaceCustom | WrappedKind::Custom,
									..
								}) => {
									let input = input_object["input"].as_str().ok_or(())?.to_string();
									if !input.is_empty() {
										stream.mark_visible();
									}
									Ok(vec![
										(
											"response.custom_tool_call_input.delta",
											responses::ResponseStreamEvent::ResponseCustomToolCallInputDelta(
												responses::ResponseCustomToolCallInputDeltaEvent {
													sequence_number: stream.sequence()?,
													output_index: block.output_index,
													item_id: block.item_id.clone(),
													delta: input.clone(),
												},
											),
										),
										(
											"response.custom_tool_call_input.done",
											responses::ResponseStreamEvent::ResponseCustomToolCallInputDone(
												responses::ResponseCustomToolCallInputDoneEvent {
													sequence_number: stream.sequence()?,
													output_index: block.output_index,
													item_id: block.item_id.clone(),
													input,
												},
											),
										),
									])
								},
								DeclaredTool::Wrapped(_) => {
									stream.mark_visible();
									Ok(vec![(
										"response.output_item.added",
										responses::ResponseStreamEvent::ResponseOutputItemAdded(
											responses::ResponseOutputItemAddedEvent {
												sequence_number: stream.sequence()?,
												output_index: block.output_index,
												item: item.clone(),
											},
										),
									)])
								},
							}
						},
						_ => Err(()),
					}
				},
				messages::MessagesStreamEvent::MessageDelta { delta, usage } => {
					if stream.message_id.is_none()
						|| stream.active_block.is_some()
						|| stream.saw_message_delta
						|| delta.stop_reason.is_none()
					{
						return Err(());
					}
					stream.saw_message_delta = true;
					stream.stop_reason = delta.stop_reason;
					stream.stop_sequence = delta.stop_sequence;
					stream.terminal_usage = Some(usage);
					Ok(Vec::new())
				},
				messages::MessagesStreamEvent::MessageStop => {
					if !stream.saw_message_delta || stream.active_block.is_some() {
						return Err(());
					}
					let initial = stream.initial_usage.clone().ok_or(())?;
					let terminal = stream.terminal_usage.clone().ok_or(())?;
					let usage = stream_usage(&initial, &terminal)?;
					let stop_reason = stream.stop_reason.ok_or(())?;
					if (matches!(stop_reason, messages::StopReason::ToolUse) && !stream.saw_tool)
						|| (stream.saw_tool
							&& matches!(
								stop_reason,
								messages::StopReason::EndTurn
									| messages::StopReason::StopSequence
									| messages::StopReason::Refusal
							)) {
						return Err(());
					}
					if match stop_reason {
						messages::StopReason::StopSequence => !stream
							.stop_sequence
							.as_ref()
							.is_some_and(|value| !value.is_empty()),
						_ => stream.stop_sequence.is_some(),
					} {
						return Err(());
					}
					stream.ensure_retained_limit(buffer_limit, &model)?;
					// Anthropic only reveals a refusal via the terminal stop_reason, but Responses
					// commits to output_text vs refusal typing as soon as content_part.added is sent.
					// Once any text has actually streamed, it cannot be retyped, so a late refusal
					// must fail the stream rather than silently relabel already-emitted content.
					if matches!(stop_reason, messages::StopReason::Refusal) && !stream.output.is_empty() {
						stream.late_refusal = true;
						return Err(());
					}
					let mut refusal_lifecycle_events = Vec::new();
					if matches!(stop_reason, messages::StopReason::Refusal) {
						let message_id = stream.message_id.clone().ok_or(())?;
						let item_id = format!("msg_{message_id}_0");
						let output_index = u32::try_from(stream.output.len()).map_err(|_| ())?;
						refusal_lifecycle_events.push((
							"response.output_item.added",
							responses::ResponseStreamEvent::ResponseOutputItemAdded(
								responses::ResponseOutputItemAddedEvent {
									sequence_number: stream.sequence()?,
									output_index,
									item: stream_message_item(
										item_id.clone(),
										String::new(),
										responses::OutputStatus::InProgress,
									),
								},
							),
						));
						refusal_lifecycle_events.push((
							"response.content_part.added",
							responses::ResponseStreamEvent::ResponseContentPartAdded(
								responses::ResponseContentPartAddedEvent {
									sequence_number: stream.sequence()?,
									item_id: item_id.clone(),
									output_index,
									content_index: 0,
									part: stream_refusal_part(String::new()),
								},
							),
						));
						refusal_lifecycle_events.push((
							"response.refusal.done",
							responses::ResponseStreamEvent::ResponseRefusalDone(
								responses::ResponseRefusalDoneEvent {
									sequence_number: stream.sequence()?,
									item_id: item_id.clone(),
									output_index,
									content_index: 0,
									refusal: String::new(),
								},
							),
						));
						refusal_lifecycle_events.push((
							"response.content_part.done",
							responses::ResponseStreamEvent::ResponseContentPartDone(
								responses::ResponseContentPartDoneEvent {
									sequence_number: stream.sequence()?,
									item_id,
									output_index,
									content_index: 0,
									part: stream_refusal_part(String::new()),
								},
							),
						));
						stream
							.retain_output(stream_empty_refusal_item(
								format!("msg_{message_id}_0"),
								responses::OutputStatus::InProgress,
							))
							.map_err(|_| ())?;
					}
					let builder = response_builder.as_ref().ok_or(())?;
					let (status, incomplete_reason) = terminal_status(stop_reason).ok_or(())?;
					let output_status = if status == "completed" {
						responses::OutputStatus::Completed
					} else {
						responses::OutputStatus::Incomplete
					};
					let mut output = std::mem::take(&mut stream.output);
					for item in &mut output {
						set_output_item_status(item, output_status)?;
					}
					let mut events = refusal_lifecycle_events;
					events.reserve(output.len() + 1);
					for (output_index, item) in output.iter().cloned().enumerate() {
						events.push((
							"response.output_item.done",
							responses::ResponseStreamEvent::ResponseOutputItemDone(
								responses::ResponseOutputItemDoneEvent {
									sequence_number: stream.sequence()?,
									output_index: u32::try_from(output_index).map_err(|_| ())?,
									item,
								},
							),
						));
					}
					let mut response = builder.response(
						if status == "completed" {
							responses::Status::Completed
						} else {
							responses::Status::Incomplete
						},
						Some(usage.clone()),
						None,
						incomplete_reason.map(|reason| responses::IncompleteDetails {
							reason: reason.to_string(),
						}),
					);
					response.output = output;
					response.service_tier = stream_service_tier(initial.service_tier.as_deref())?;
					let sequence_number = stream.sequence()?;
					let event = if status == "completed" {
						responses::ResponseStreamEvent::ResponseCompleted(responses::ResponseCompletedEvent {
							sequence_number,
							response,
						})
					} else {
						responses::ResponseStreamEvent::ResponseIncomplete(responses::ResponseIncompleteEvent {
							sequence_number,
							response,
						})
					};
					stream.terminal_ready = true;
					events.push((
						if status == "completed" {
							"response.completed"
						} else {
							"response.incomplete"
						},
						event,
					));
					Ok(events)
				},
				messages::MessagesStreamEvent::Ping => {
					// A ping is a content-free keepalive Anthropic may send at any point in the
					// stream, including before message_start while a request is queued -- unlike
					// every other event type, it carries no state that depends on message_id
					// already being set, so there is nothing to validate here.
					Ok(Vec::new())
				},
			}
		})();
		let result = result.and_then(|events| {
			stream.ensure_retained_limit(buffer_limit, &model)?;
			Ok(events)
		});
		let output = match result {
			Ok(events) => {
				if stream.terminal_ready {
					if commit_stream_telemetry(&stream, &log).is_err() {
						stream.sequence_number = sequence_checkpoint;
						stream.error_event()
					} else {
						stream.terminated = true;
						events
					}
				} else {
					events
				}
			},
			Err(()) => {
				stream.sequence_number = sequence_checkpoint;
				stream.error_event()
			},
		};
		(output, stream.terminated)
	})
}

fn invalid_response() -> AIError {
	AIError::InvalidResponse(strng::literal!("invalid Anthropic Messages response"))
}

fn validate_top_level(raw: &serde_json::Value) -> Result<State, AIError> {
	let object = raw.as_object().ok_or_else(|| {
		AIError::UnsupportedConversion(strng::literal!("unsupported Responses request"))
	})?;
	let known = [
		"background",
		"client_metadata",
		"conversation",
		"include",
		"input",
		"instructions",
		"logprobs",
		"max_output_tokens",
		"max_tool_calls",
		"metadata",
		"model",
		"parallel_tool_calls",
		"previous_response_id",
		"prompt",
		"prompt_cache_key",
		"prompt_cache_retention",
		"reasoning",
		"safety_identifier",
		"service_tier",
		"store",
		"stream",
		"stream_options",
		"temperature",
		"text",
		"tool_choice",
		"tools",
		"top_logprobs",
		"top_p",
		"truncation",
		"user",
		"vendor_extensions",
	];
	if has_unknown_field(object, &known) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses request field"
		)));
	}
	if ["user", "safety_identifier"].into_iter().any(|field| {
		!matches!(
			object.get(field),
			None | Some(serde_json::Value::Null | serde_json::Value::String(_))
		)
	}) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses user identifier"
		)));
	}
	if object.get("store") != Some(&serde_json::Value::Bool(false)) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses requests must set store to false"
		)));
	}
	if ["previous_response_id", "conversation", "prompt"]
		.into_iter()
		.any(|field| object.get(field).is_some_and(has_effect))
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses server-side conversation state is unsupported"
		)));
	}
	if [
		"client_metadata",
		"metadata",
		"prompt_cache_key",
		"prompt_cache_retention",
	]
	.into_iter()
	.any(|field| object.get(field).is_some_and(has_effect))
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses configuration is unsupported"
		)));
	}
	if !absent_or_false(object.get("background")) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses background mode is unsupported"
		)));
	}
	let stream = object
		.get("stream")
		.and_then(serde_json::Value::as_bool)
		.unwrap_or(false);
	let valid_stream_options = match object.get("stream_options") {
		None => !stream,
		Some(serde_json::Value::Object(options)) => {
			options.len() == 1
				&& options.get("include_obfuscation") == Some(&serde_json::Value::Bool(false))
		},
		Some(_) => false,
	};
	if !valid_stream_options {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses stream options must explicitly disable obfuscation"
		)));
	}
	match object.get("logprobs") {
		None | Some(serde_json::Value::Null | serde_json::Value::Bool(false)) => {},
		Some(serde_json::Value::Bool(true)) => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"unsupported Responses request option"
			)));
		},
		Some(_) => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"unsupported Responses logprobs"
			)));
		},
	}
	if ["max_tool_calls", "service_tier", "top_logprobs"]
		.into_iter()
		.any(|field| object.get(field).is_some_and(has_effect))
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses request option"
		)));
	}
	let valid_truncation = match object.get("truncation") {
		None => true,
		Some(serde_json::Value::String(truncation)) => truncation == "disabled",
		_ => false,
	};
	if !valid_truncation {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses truncation"
		)));
	}
	if let Some(text) = object.get("text").and_then(serde_json::Value::as_object)
		&& has_unknown_field(text, &["format", "verbosity"])
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses text option"
		)));
	}
	if object
		.get("text")
		.and_then(serde_json::Value::as_object)
		.and_then(|text| text.get("verbosity"))
		.is_some_and(has_effect)
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses text verbosity is unsupported"
		)));
	}
	if object.get("vendor_extensions").is_some_and(has_effect) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses vendor extensions are unsupported"
		)));
	}
	let valid_reasoning = match object.get("reasoning") {
		None => true,
		Some(serde_json::Value::Object(reasoning)) => {
			reasoning.len() == 1
				&& reasoning.get("effort").and_then(serde_json::Value::as_str) == Some("none")
		},
		Some(_) => false,
	};
	if !valid_reasoning {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses reasoning is unsupported"
		)));
	}
	if object.contains_key("include") {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses include is unsupported"
		)));
	}

	let mut typed = raw.clone();
	let typed_object = typed
		.as_object_mut()
		.expect("request object validated above");
	typed_object.retain(|key, _| known.contains(&key.as_str()));
	typed_object.remove("client_metadata");
	typed_object.insert("input".to_string(), serde_json::Value::Array(Vec::new()));
	typed_object.insert("tools".to_string(), serde_json::Value::Array(Vec::new()));
	typed_object.insert("tool_choice".to_string(), serde_json::Value::Null);
	serde_json::from_value::<types::responses::typed::CreateResponse>(typed).map_err(|_| {
		AIError::UnsupportedConversion(strng::literal!("unsupported Responses request field"))
	})?;
	Ok(State {
		tools: HashMap::new(),
	})
}

fn has_effect(value: &serde_json::Value) -> bool {
	match value {
		serde_json::Value::Null | serde_json::Value::Bool(false) => false,
		serde_json::Value::String(value) => !value.is_empty(),
		serde_json::Value::Array(values) => values.iter().any(has_effect),
		serde_json::Value::Object(values) => values.values().any(has_effect),
		_ => true,
	}
}

fn has_unknown_field(object: &serde_json::Map<String, serde_json::Value>, known: &[&str]) -> bool {
	object.keys().any(|key| !known.contains(&key.as_str()))
}

fn absent_or_false(value: Option<&serde_json::Value>) -> bool {
	matches!(value, None | Some(serde_json::Value::Bool(false)))
}

fn translate_tools(
	raw: &serde_json::Value,
	state: &mut State,
) -> Result<Vec<messages::Tool>, AIError> {
	let Some(value) = raw.get("tools") else {
		return Ok(Vec::new());
	};
	let definitions = value.as_array().ok_or_else(invalid_tool_declaration)?;
	let mut tools = Vec::new();
	let mut declarations = HashSet::new();
	for definition in definitions {
		let object = definition
			.as_object()
			.ok_or_else(invalid_tool_declaration)?;
		match object.get("type").and_then(serde_json::Value::as_str) {
			Some("function") => add_function_tool(object, None, &mut tools, state, &mut declarations)?,
			Some("namespace") => {
				if has_unknown_field(object, &["type", "name", "description", "tools"]) {
					return Err(invalid_tool_declaration());
				}
				let namespace = required_nonempty_string(object, "name")?;
				if !declarations.insert(format!("namespace:{namespace}")) {
					return Err(invalid_tool_declaration());
				}
				let namespace_description = object
					.get("description")
					.and_then(serde_json::Value::as_str)
					.ok_or_else(invalid_tool_declaration)?;
				let children = object
					.get("tools")
					.and_then(serde_json::Value::as_array)
					.ok_or_else(invalid_tool_declaration)?;
				for child in children {
					let child = child.as_object().ok_or_else(invalid_tool_declaration)?;
					match child.get("type").and_then(serde_json::Value::as_str) {
						Some("function") => add_function_tool(
							child,
							Some((namespace, namespace_description)),
							&mut tools,
							state,
							&mut declarations,
						)?,
						Some("custom") => add_custom_tool(
							child,
							Some((namespace, namespace_description)),
							&mut tools,
							state,
							&mut declarations,
						)?,
						_ => return Err(invalid_tool_declaration()),
					}
				}
			},
			Some("custom") => add_custom_tool(object, None, &mut tools, state, &mut declarations)?,
			Some("local_shell") => add_unit_wrapped_tool(
				object,
				WrappedKind::LocalShell,
				"local_shell",
				&mut tools,
				state,
				&mut declarations,
			)?,
			Some("shell") => {
				add_unit_wrapped_tool(
					object,
					WrappedKind::Shell,
					"shell",
					&mut tools,
					state,
					&mut declarations,
				)?;
			},
			Some("apply_patch") => add_unit_wrapped_tool(
				object,
				WrappedKind::ApplyPatch,
				"apply_patch",
				&mut tools,
				state,
				&mut declarations,
			)?,
			_ => return Err(invalid_tool_declaration()),
		}
	}
	Ok(tools)
}

fn add_function_tool(
	object: &serde_json::Map<String, serde_json::Value>,
	namespace: Option<(&str, &str)>,
	tools: &mut Vec<messages::Tool>,
	state: &mut State,
	declarations: &mut HashSet<String>,
) -> Result<(), AIError> {
	if has_unknown_field(
		object,
		&[
			"type",
			"name",
			"description",
			"parameters",
			"strict",
			"defer_loading",
		],
	) || !absent_or_false(object.get("strict"))
		|| !absent_or_false(object.get("defer_loading"))
	{
		return Err(invalid_tool_declaration());
	}
	let name = required_nonempty_string(object, "name")?;
	let description = optional_string(object, "description")?;
	let parameters = object
		.get("parameters")
		.cloned()
		.unwrap_or_else(|| serde_json::json!({"type": "object"}));
	if !parameters.is_object() {
		return Err(invalid_tool_declaration());
	}
	if let Some((namespace, namespace_description)) = namespace {
		let key = format!("namespace_function:{namespace}:{name}");
		if !declarations.insert(key) {
			return Err(invalid_tool_declaration());
		}
		add_wrapped_tool(
			WrappedKind::NamespaceFunction,
			"namespace_function",
			name,
			Some(namespace),
			Some(wrapped_tool_description(
				Some((namespace, namespace_description)),
				name,
				description,
			)),
			wrapper_schema(WrappedKind::NamespaceFunction.field_name(), parameters),
			tools,
			state,
		);
	} else {
		if !valid_anthropic_tool_name(name)
			|| name.starts_with("agentgateway__responses__")
			|| !declarations.insert(format!("function:{name}"))
		{
			return Err(invalid_tool_declaration());
		}
		tools.push(messages::Tool {
			name: name.to_string(),
			description,
			input_schema: parameters,
			cache_control: None,
		});
		state.tools.insert(name.to_string(), DeclaredTool::Function);
	}
	Ok(())
}

fn add_custom_tool(
	object: &serde_json::Map<String, serde_json::Value>,
	namespace: Option<(&str, &str)>,
	tools: &mut Vec<messages::Tool>,
	state: &mut State,
	declarations: &mut HashSet<String>,
) -> Result<(), AIError> {
	if has_unknown_field(
		object,
		&["type", "name", "description", "format", "defer_loading"],
	) || !absent_or_false(object.get("defer_loading"))
	{
		return Err(invalid_tool_declaration());
	}
	match object.get("format") {
		None => {},
		Some(serde_json::Value::Object(format))
			if format.len() == 1
				&& format.get("type").and_then(serde_json::Value::as_str) == Some("text") => {},
		_ => return Err(invalid_tool_declaration()),
	}
	let name = required_nonempty_string(object, "name")?;
	let description = optional_string(object, "description")?;
	let (kind, token, key, namespace_name) = if let Some((namespace, _)) = namespace {
		(
			WrappedKind::NamespaceCustom,
			"namespace_custom",
			format!("namespace_custom:{namespace}:{name}"),
			Some(namespace),
		)
	} else {
		(
			WrappedKind::Custom,
			"custom",
			format!("custom:{name}"),
			None,
		)
	};
	if !declarations.insert(key) {
		return Err(invalid_tool_declaration());
	}
	let field_name = kind.field_name();
	add_wrapped_tool(
		kind,
		token,
		name,
		namespace_name,
		Some(wrapped_tool_description(namespace, name, description)),
		wrapper_schema(field_name, serde_json::json!({"type": "string"})),
		tools,
		state,
	);
	Ok(())
}

fn add_unit_wrapped_tool(
	object: &serde_json::Map<String, serde_json::Value>,
	kind: WrappedKind,
	token: &str,
	tools: &mut Vec<messages::Tool>,
	state: &mut State,
	declarations: &mut HashSet<String>,
) -> Result<(), AIError> {
	if kind == WrappedKind::Shell {
		if has_unknown_field(object, &["type", "environment"]) {
			return Err(invalid_tool_declaration());
		}
		if !object
			.get("environment")
			.is_some_and(shell_environment_is_local)
		{
			return Err(invalid_tool_declaration());
		}
	}
	if kind != WrappedKind::Shell && has_unknown_field(object, &["type"])
		|| !declarations.insert(token.to_string())
	{
		return Err(invalid_tool_declaration());
	}
	let (description, input_schema) = match kind {
		WrappedKind::LocalShell => (
			"Execute one command on the client's local computer. Provide command as an argv array, env as string environment variables, and optional timeout_ms, user, and working_directory.",
			local_shell_schema(),
		),
		WrappedKind::Shell => (
			"Execute one or more shell command strings in order on the client's local computer. Optional timeout_ms and max_output_length values limit total execution time and captured output.",
			shell_schema(),
		),
		WrappedKind::ApplyPatch => (
			"Create, delete, or update one file in the client's local workspace. Create and update operations use path and diff. Delete operations use path.",
			apply_patch_schema(),
		),
		_ => unreachable!("only unit wrapped tools use this helper"),
	};
	add_wrapped_tool(
		kind,
		token,
		token,
		None,
		Some(description.to_string()),
		input_schema,
		tools,
		state,
	);
	Ok(())
}

fn local_shell_schema() -> serde_json::Value {
	wrapper_schema(
		WrappedKind::LocalShell.field_name(),
		serde_json::json!({
			"type":"object",
			"properties":{
				"command":{"type":"array","items":{"type":"string"}},
				"env":{"type":"object","additionalProperties":{"type":"string"}},
				"timeout_ms":{"type":["integer","null"],"minimum":0},
				"user":{"type":["string","null"]},
				"working_directory":{"type":["string","null"]}
			},
			"required":["command","env"],
			"additionalProperties":false
		}),
	)
}

fn shell_schema() -> serde_json::Value {
	wrapper_schema(
		WrappedKind::Shell.field_name(),
		serde_json::json!({
			"type":"object",
			"properties":{
				"commands":{"type":"array","items":{"type":"string"}},
				"timeout_ms":{"type":["integer","null"],"minimum":0},
				"max_output_length":{"type":["integer","null"],"minimum":0}
			},
			"required":["commands"],
			"additionalProperties":false
		}),
	)
}

fn apply_patch_schema() -> serde_json::Value {
	wrapper_schema(
		WrappedKind::ApplyPatch.field_name(),
		serde_json::json!({
			"oneOf":[
				{
					"type":"object",
					"properties":{"type":{"const":"create_file"},"path":{"type":"string"},"diff":{"type":"string"}},
					"required":["type","path","diff"],
					"additionalProperties":false
				},
				{
					"type":"object",
					"properties":{"type":{"const":"delete_file"},"path":{"type":"string"}},
					"required":["type","path"],
					"additionalProperties":false
				},
				{
					"type":"object",
					"properties":{"type":{"const":"update_file"},"path":{"type":"string"},"diff":{"type":"string"}},
					"required":["type","path","diff"],
					"additionalProperties":false
				}
			]
		}),
	)
}

#[allow(clippy::too_many_arguments)]
fn add_wrapped_tool(
	kind: WrappedKind,
	token: &str,
	name: &str,
	namespace: Option<&str>,
	description: Option<String>,
	input_schema: serde_json::Value,
	tools: &mut Vec<messages::Tool>,
	state: &mut State,
) {
	let upstream_name = format!("agentgateway__responses__{token}_{}", tools.len());
	tools.push(messages::Tool {
		name: upstream_name.clone(),
		description,
		input_schema,
		cache_control: None,
	});
	state.tools.insert(
		upstream_name,
		DeclaredTool::Wrapped(WrappedTool {
			kind,
			name: name.to_string(),
			namespace: namespace.map(ToOwned::to_owned),
		}),
	);
}

fn wrapped_tool_description(
	namespace: Option<(&str, &str)>,
	name: &str,
	description: Option<String>,
) -> String {
	let qualified_name = namespace
		.map(|(namespace, _)| format!("{namespace}.{name}"))
		.unwrap_or_else(|| name.to_string());
	let mut parts = vec![format!("Responses tool {qualified_name}.")];
	if let Some((_, description)) = namespace
		&& !description.is_empty()
	{
		parts.push(description.to_string());
	}
	if let Some(description) = description
		&& !description.is_empty()
	{
		parts.push(description);
	}
	parts.join(" ")
}

fn wrapper_schema(field: &str, mut value: serde_json::Value) -> serde_json::Value {
	rewrite_wrapped_schema_refs(&mut value, &format!("/properties/{field}"));
	serde_json::json!({
		"type": "object",
		"properties": {(field): value},
		"required": [field],
		"additionalProperties": false
	})
}

// Nesting an arbitrary caller-supplied schema under `properties.<field>` moves it one level
// deeper in the document, so a root-relative JSON Pointer `$ref` inside it (e.g. "#/$defs/Foo",
// or a bare "#" self-reference) would otherwise resolve against the new wrapper root instead of
// the schema's own root. Rewrite those to keep pointing at the same target; the same applies to
// "$recursiveRef" (draft 2019-09) whenever it is used in pointer-shaped form ("#" or "#/...").
// A subschema that declares its own "$id" establishes a new resolution base -- stop descending
// into it so its refs, already relative to that "$id", aren't corrupted.
//
// Only descends into keywords the JSON Schema core/applicator vocabulary defines as holding a
// nested schema (or array/map of them) -- e.g. "properties", "items", "allOf" -- never into
// keywords that hold plain data which might merely look like a schema, such as "const", "enum",
// or "default". Recursing into those would corrupt a literal value that happens to contain a
// "$ref"-shaped object.
fn rewrite_wrapped_schema_refs(schema: &mut serde_json::Value, prefix: &str) {
	let serde_json::Value::Object(map) = schema else {
		return;
	};
	if map.contains_key("$id") {
		return;
	}
	for ref_key in ["$ref", "$recursiveRef"] {
		if let Some(serde_json::Value::String(r)) = map.get_mut(ref_key)
			&& let Some(rest) = r.strip_prefix('#')
			&& (rest.is_empty() || rest.starts_with('/'))
		{
			*r = format!("#{prefix}{rest}");
		}
	}
	// Keywords whose value is always a single nested schema.
	for key in [
		"additionalProperties",
		"unevaluatedProperties",
		"unevaluatedItems",
		"propertyNames",
		"contains",
		"if",
		"then",
		"else",
		"not",
	] {
		if let Some(v) = map.get_mut(key) {
			rewrite_wrapped_schema_refs(v, prefix);
		}
	}
	// `items` is a single schema in modern drafts, or an array of per-position schemas (tuple
	// validation) in draft-07 and earlier.
	if let Some(v) = map.get_mut("items") {
		match v {
			serde_json::Value::Array(items) => {
				for item in items {
					rewrite_wrapped_schema_refs(item, prefix);
				}
			},
			_ => rewrite_wrapped_schema_refs(v, prefix),
		}
	}
	// Keywords whose value is an array of nested schemas.
	for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
		if let Some(serde_json::Value::Array(items)) = map.get_mut(key) {
			for item in items {
				rewrite_wrapped_schema_refs(item, prefix);
			}
		}
	}
	// Keywords whose value is a map from arbitrary keys to nested schemas -- only the values,
	// never the keys, are schemas.
	for key in [
		"properties",
		"patternProperties",
		"$defs",
		"definitions",
		"dependentSchemas",
	] {
		if let Some(serde_json::Value::Object(props)) = map.get_mut(key) {
			for v in props.values_mut() {
				rewrite_wrapped_schema_refs(v, prefix);
			}
		}
	}
}

fn required_nonempty_string<'a>(
	object: &'a serde_json::Map<String, serde_json::Value>,
	field: &str,
) -> Result<&'a str, AIError> {
	object
		.get(field)
		.and_then(serde_json::Value::as_str)
		.filter(|value| !value.is_empty())
		.ok_or_else(invalid_tool_declaration)
}

fn optional_string(
	object: &serde_json::Map<String, serde_json::Value>,
	field: &str,
) -> Result<Option<String>, AIError> {
	match object.get(field) {
		None => Ok(None),
		Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
		Some(_) => Err(invalid_tool_declaration()),
	}
}

fn valid_anthropic_tool_name(name: &str) -> bool {
	!name.is_empty()
		&& name.len() <= 64
		&& name
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn invalid_tool_declaration() -> AIError {
	AIError::UnsupportedConversion(strng::literal!("invalid Responses tool declaration"))
}

fn translate_tool_choice(
	raw: &serde_json::Value,
	state: &State,
) -> Result<Option<messages::ToolChoice>, AIError> {
	let disable_parallel_tool_use =
		(raw.get("parallel_tool_calls") == Some(&serde_json::Value::Bool(false))).then_some(true);
	let Some(choice) = raw.get("tool_choice").filter(|choice| !choice.is_null()) else {
		return Ok(
			(disable_parallel_tool_use.is_some() && !state.tools.is_empty()).then_some(
				messages::ToolChoice::Auto {
					disable_parallel_tool_use,
				},
			),
		);
	};
	let translated = match choice {
		serde_json::Value::String(mode) if mode == "auto" => messages::ToolChoice::Auto {
			disable_parallel_tool_use,
		},
		serde_json::Value::String(mode) if mode == "required" => {
			if state.tools.is_empty() {
				return Err(invalid_tool_choice());
			}
			messages::ToolChoice::Any {
				disable_parallel_tool_use,
			}
		},
		serde_json::Value::String(mode) if mode == "none" => messages::ToolChoice::None {},
		serde_json::Value::Object(object) => {
			let kind = object.get("type").and_then(serde_json::Value::as_str);
			let upstream_name = match kind {
				Some("function") if !has_unknown_field(object, &["type", "name"]) => {
					let name = required_choice_name(object)?;
					find_named_tool(state, name, |tool| {
						matches!(tool, DeclaredTool::Function)
							|| matches!(tool, DeclaredTool::Wrapped(wrapped) if wrapped.kind == WrappedKind::NamespaceFunction)
					})?
				},
				Some("custom") if !has_unknown_field(object, &["type", "name"]) => {
					let name = required_choice_name(object)?;
					find_named_tool(
						state,
						name,
						|tool| matches!(tool, DeclaredTool::Wrapped(wrapped) if matches!(wrapped.kind, WrappedKind::Custom | WrappedKind::NamespaceCustom)),
					)?
				},
				Some("local_shell") if object.len() == 1 => {
					find_wrapped_kind(state, WrappedKind::LocalShell)?
				},
				Some("shell") if object.len() == 1 => find_wrapped_kind(state, WrappedKind::Shell)?,
				Some("apply_patch") if object.len() == 1 => {
					find_wrapped_kind(state, WrappedKind::ApplyPatch)?
				},
				_ => return Err(invalid_tool_choice()),
			};
			messages::ToolChoice::Tool {
				name: upstream_name,
				disable_parallel_tool_use,
			}
		},
		_ => return Err(invalid_tool_choice()),
	};
	Ok(Some(translated))
}

fn required_choice_name(
	object: &serde_json::Map<String, serde_json::Value>,
) -> Result<&str, AIError> {
	object
		.get("name")
		.and_then(serde_json::Value::as_str)
		.filter(|name| !name.is_empty())
		.ok_or_else(invalid_tool_choice)
}

fn find_named_tool(
	state: &State,
	name: &str,
	matches_kind: impl Fn(&DeclaredTool) -> bool,
) -> Result<String, AIError> {
	state
		.tools
		.iter()
		.filter(|(upstream, tool)| {
			matches_kind(tool)
				&& match tool {
					DeclaredTool::Function => upstream.as_str() == name,
					DeclaredTool::Wrapped(wrapped) => wrapped.name == name,
				}
		})
		.exactly_one()
		.map(|(upstream, _)| upstream.clone())
		.map_err(|_| invalid_tool_choice())
}

fn find_wrapped_kind(state: &State, kind: WrappedKind) -> Result<String, AIError> {
	state
		.tools
		.iter()
		.filter(|(_, tool)| matches!(tool, DeclaredTool::Wrapped(wrapped) if wrapped.kind == kind))
		.exactly_one()
		.map(|(upstream, _)| upstream.clone())
		.map_err(|_| invalid_tool_choice())
}

fn invalid_tool_choice() -> AIError {
	AIError::UnsupportedConversion(strng::literal!("invalid Responses tool choice"))
}

fn responses_output_format(
	raw: &serde_json::Value,
) -> Result<Option<messages::OutputFormat>, AIError> {
	let Some(format) = raw
		.get("text")
		.and_then(serde_json::Value::as_object)
		.and_then(|text| text.get("format"))
	else {
		return Ok(None);
	};
	if !has_effect(format) {
		return Ok(None);
	}
	let format = format.as_object().ok_or_else(|| {
		AIError::UnsupportedConversion(strng::literal!("Responses output formats are unsupported"))
	})?;
	match format.get("type").and_then(serde_json::Value::as_str) {
		Some("text") => {
			if has_unknown_field(format, &["type"]) {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"unsupported Responses text format option"
				)));
			}
			Ok(None)
		},
		Some("json_schema") => {
			if has_unknown_field(format, &["type", "name", "description", "schema", "strict"]) {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"unsupported Responses text format option"
				)));
			}
			if format.get("strict") != Some(&serde_json::Value::Bool(true)) {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"Responses JSON schema output requires strict true"
				)));
			}
			let mut schema = format.get("schema").cloned().ok_or_else(|| {
				AIError::UnsupportedConversion(strng::literal!(
					"Responses JSON schema output requires a schema"
				))
			})?;
			if let Some(description) = format
				.get("description")
				.and_then(serde_json::Value::as_str)
				&& let Some(object) = schema.as_object_mut()
				&& !object.contains_key("description")
			{
				object.insert(
					"description".to_string(),
					serde_json::Value::String(description.to_string()),
				);
			}
			Ok(Some(messages::OutputFormat::JsonSchema { schema }))
		},
		_ => Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses output formats are unsupported"
		))),
	}
}

fn translate_input(
	input: &serde_json::Value,
	instructions: Option<&str>,
	state: &State,
) -> Result<(Vec<messages::Message>, Option<messages::SystemPrompt>), AIError> {
	if instructions.is_some_and(str::is_empty) {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses instructions must not be empty"
		)));
	}
	match input {
		serde_json::Value::String(text) if text.is_empty() => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"Responses input must not be empty"
			)));
		},
		serde_json::Value::Array(items) if items.is_empty() => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"Responses input must not be empty"
			)));
		},
		_ => {},
	}
	let mut output = Vec::new();
	let mut calls = HashMap::new();
	let mut completed_outputs = HashSet::new();
	let mut system = instructions
		.map(ToOwned::to_owned)
		.into_iter()
		.collect::<Vec<_>>();
	match input {
		serde_json::Value::String(text) => {
			push_message(&mut output, messages::Role::User, vec![text.clone()]);
		},
		serde_json::Value::Array(items) => {
			for item in items {
				let object = item.as_object().ok_or_else(|| {
					AIError::UnsupportedConversion(strng::literal!("unsupported Responses input item"))
				})?;
				if let Some((role, block)) =
					translate_tool_history_item(object, state, &mut calls, &mut completed_outputs)?
				{
					push_blocks(&mut output, role, vec![block]);
					continue;
				}
				if object.get("type").and_then(serde_json::Value::as_str) == Some("reasoning") {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"Responses reasoning history is unsupported"
					)));
				}
				if object
					.get("type")
					.is_some_and(|kind| kind.as_str() != Some("message"))
				{
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"unsupported Responses input item"
					)));
				}
				if object.get("id").is_some_and(|id| !id.is_string()) {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"unsupported Responses message id"
					)));
				}
				let role = object
					.get("role")
					.and_then(serde_json::Value::as_str)
					.ok_or_else(|| {
						AIError::UnsupportedConversion(strng::literal!("Responses message role is required"))
					})?;
				let known = if role == "assistant" {
					&["type", "role", "content", "id", "phase", "status"][..]
				} else {
					&["type", "role", "content", "id"][..]
				};
				if has_unknown_field(object, known) {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"unsupported Responses input item field"
					)));
				}
				let content = object.get("content").ok_or_else(|| {
					AIError::UnsupportedConversion(strng::literal!("Responses message content is required"))
				})?;
				match role {
					"system" | "developer" => {
						if !output.is_empty() {
							return Err(AIError::UnsupportedConversion(strng::literal!(
								"Responses system messages must precede conversation messages"
							)));
						}
						system.extend(text_content(content, "input_text")?);
					},
					"user" => push_blocks(&mut output, messages::Role::User, user_content(content)?),
					"assistant" => {
						validate_assistant(object)?;
						push_message(
							&mut output,
							messages::Role::Assistant,
							assistant_text_content(content)?,
						);
					},
					_ => {
						return Err(AIError::UnsupportedConversion(strng::literal!(
							"unsupported Responses message role"
						)));
					},
				}
			}
		},
		_ => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"unsupported Responses input"
			)));
		},
	}
	validate_tool_result_order(&output)?;
	let system = (!system.is_empty()).then(|| {
		messages::SystemPrompt::Blocks(
			system
				.into_iter()
				.map(|text| messages::SystemContentBlock::Text {
					text,
					cache_control: None,
				})
				.collect(),
		)
	});
	Ok((output, system))
}

fn validate_tool_result_order(history: &[messages::Message]) -> Result<(), AIError> {
	let mut expected_results: Option<HashSet<String>> = None;
	for message in history {
		match message.role {
			messages::Role::Assistant => {
				if expected_results.is_some() {
					return Err(invalid_tool_history());
				}
				let calls = message
					.content
					.iter()
					.filter_map(|block| match block {
						messages::ContentBlock::ToolUse { id, .. } => Some(id.clone()),
						_ => None,
					})
					.collect::<HashSet<_>>();
				if !calls.is_empty() {
					expected_results = Some(calls);
				}
			},
			messages::Role::User => {
				let mut results = HashSet::new();
				let mut saw_other_content = false;
				for block in &message.content {
					if let messages::ContentBlock::ToolResult { tool_use_id, .. } = block {
						if saw_other_content || !results.insert(tool_use_id.clone()) {
							return Err(invalid_tool_history());
						}
					} else {
						saw_other_content = true;
					}
				}
				match expected_results.take() {
					Some(expected) if expected == results => {},
					Some(_) => return Err(invalid_tool_history()),
					None if !results.is_empty() => return Err(invalid_tool_history()),
					None => {},
				}
			},
			messages::Role::System => return Err(invalid_tool_history()),
		}
	}
	if expected_results.is_some() {
		return Err(invalid_tool_history());
	}
	Ok(())
}

fn translate_tool_history_item(
	object: &serde_json::Map<String, serde_json::Value>,
	state: &State,
	calls: &mut HashMap<String, DeclaredTool>,
	completed_outputs: &mut HashSet<String>,
) -> Result<Option<(messages::Role, messages::ContentBlock)>, AIError> {
	let Some(kind) = object.get("type").and_then(serde_json::Value::as_str) else {
		return Ok(None);
	};
	let (role, block) = match kind {
		"function_call" => {
			if has_unknown_field(
				object,
				&[
					"type",
					"arguments",
					"call_id",
					"namespace",
					"name",
					"id",
					"status",
				],
			) {
				return Err(invalid_tool_history());
			}
			validate_terminal_call_status(object.get("status"))?;
			validate_optional_string(object, "id")?;
			let call_id = history_call_id(object, "call_id")?;
			let name = history_string(object, "name")?;
			let namespace = optional_history_string(object, "namespace")?;
			let arguments = object
				.get("arguments")
				.and_then(serde_json::Value::as_str)
				.and_then(|arguments| serde_json::from_str::<serde_json::Value>(arguments).ok())
				.filter(serde_json::Value::is_object)
				.ok_or_else(invalid_tool_history)?;
			let (upstream_name, declared) =
				find_call_declaration(state, name, namespace, WrappedKind::NamespaceFunction, true)?;
			let input = match &declared {
				DeclaredTool::Function => arguments,
				DeclaredTool::Wrapped(_) => serde_json::json!({"arguments": arguments}),
			};
			record_call(calls, call_id, declared)?;
			(
				messages::Role::Assistant,
				messages::ContentBlock::ToolUse {
					id: call_id.to_string(),
					name: upstream_name,
					input,
					caller: None,
					cache_control: None,
				},
			)
		},
		"custom_tool_call" => {
			if has_unknown_field(
				object,
				&[
					"type",
					"call_id",
					"namespace",
					"input",
					"name",
					"id",
					"status",
				],
			) {
				return Err(invalid_tool_history());
			}
			validate_terminal_call_status(object.get("status"))?;
			history_call_id(object, "id")?;
			let call_id = history_call_id(object, "call_id")?;
			let name = history_string(object, "name")?;
			let namespace = optional_history_string(object, "namespace")?;
			let input = history_string(object, "input")?;
			let expected_kind = if namespace.is_some() {
				WrappedKind::NamespaceCustom
			} else {
				WrappedKind::Custom
			};
			let (upstream_name, declared) =
				find_call_declaration(state, name, namespace, expected_kind, false)?;
			record_call(calls, call_id, declared)?;
			(
				messages::Role::Assistant,
				messages::ContentBlock::ToolUse {
					id: call_id.to_string(),
					name: upstream_name,
					input: serde_json::json!({"input": input}),
					caller: None,
					cache_control: None,
				},
			)
		},
		"local_shell_call" => {
			tool_action_call(object, state, calls, WrappedKind::LocalShell, "action")?
		},
		"shell_call" => tool_action_call(object, state, calls, WrappedKind::Shell, "action")?,
		"apply_patch_call" => {
			tool_action_call(object, state, calls, WrappedKind::ApplyPatch, "operation")?
		},
		"function_call_output" => {
			if has_unknown_field(object, &["type", "call_id", "output", "id", "status"]) {
				return Err(invalid_tool_history());
			}
			validate_optional_string(object, "id")?;
			tool_output(
				object,
				"call_id",
				calls,
				completed_outputs,
				|declared| {
					matches!(declared, DeclaredTool::Function)
						|| matches!(declared, DeclaredTool::Wrapped(wrapped) if wrapped.kind == WrappedKind::NamespaceFunction)
				},
				tool_result_content(object.get("output"))?,
				function_output_status_is_error(object.get("status"))?,
			)?
		},
		"custom_tool_call_output" => {
			if has_unknown_field(object, &["type", "call_id", "output", "id"])
				|| object.contains_key("status")
			{
				return Err(invalid_tool_history());
			}
			validate_optional_string(object, "id")?;
			tool_output(
				object,
				"call_id",
				calls,
				completed_outputs,
				|declared| matches!(declared, DeclaredTool::Wrapped(wrapped) if matches!(wrapped.kind, WrappedKind::Custom | WrappedKind::NamespaceCustom)),
				tool_result_content(object.get("output"))?,
				false,
			)?
		},
		"local_shell_call_output" => {
			if has_unknown_field(object, &["type", "id", "output", "status"])
				|| !matches!(object.get("output"), Some(serde_json::Value::String(_)))
			{
				return Err(invalid_tool_history());
			}
			tool_output(
				object,
				"id",
				calls,
				completed_outputs,
				|declared| matches!(declared, DeclaredTool::Wrapped(wrapped) if wrapped.kind == WrappedKind::LocalShell),
				tool_result_content(object.get("output"))?,
				function_output_status_is_error(object.get("status"))?,
			)?
		},
		"shell_call_output" => {
			if has_unknown_field(
				object,
				&["type", "id", "call_id", "output", "max_output_length"],
			) {
				return Err(invalid_tool_history());
			}
			validate_optional_string(object, "id")?;
			if !optional_u64(object.get("max_output_length")) {
				return Err(invalid_tool_history());
			}
			let output = object
				.get("output")
				.and_then(serde_json::Value::as_array)
				.ok_or_else(invalid_tool_history)?;
			let mut failed = false;
			for content in output {
				let content = content.as_object().ok_or_else(invalid_tool_history)?;
				if has_unknown_field(content, &["stdout", "stderr", "outcome"])
					|| !matches!(content.get("stdout"), Some(serde_json::Value::String(_)))
					|| !matches!(content.get("stderr"), Some(serde_json::Value::String(_)))
				{
					return Err(invalid_tool_history());
				}
				let outcome = content
					.get("outcome")
					.and_then(serde_json::Value::as_object)
					.ok_or_else(invalid_tool_history)?;
				match outcome.get("type").and_then(serde_json::Value::as_str) {
					Some("timeout") if outcome.len() == 1 => failed = true,
					Some("exit")
						if outcome.len() == 2
							&& outcome
								.get("exit_code")
								.and_then(serde_json::Value::as_i64)
								.and_then(|code| i32::try_from(code).ok())
								.is_some() =>
					{
						failed |= outcome["exit_code"].as_i64() != Some(0);
					},
					_ => return Err(invalid_tool_history()),
				}
			}
			let content = messages::ToolResultContent::Text(
				serde_json::to_string(output).map_err(AIError::RequestMarshal)?,
			);
			tool_output(
				object,
				"call_id",
				calls,
				completed_outputs,
				|declared| matches!(declared, DeclaredTool::Wrapped(wrapped) if wrapped.kind == WrappedKind::Shell),
				content,
				failed,
			)?
		},
		"apply_patch_call_output" => {
			if has_unknown_field(object, &["type", "id", "call_id", "status", "output"])
				|| !matches!(
					object.get("output"),
					None | Some(serde_json::Value::Null | serde_json::Value::String(_))
				) {
				return Err(invalid_tool_history());
			}
			validate_optional_string(object, "id")?;
			let status = object
				.get("status")
				.and_then(serde_json::Value::as_str)
				.ok_or_else(invalid_tool_history)?;
			let failed = match status {
				"completed" => false,
				"failed" => true,
				_ => return Err(invalid_tool_history()),
			};
			let content = messages::ToolResultContent::Text(
				object
					.get("output")
					.and_then(serde_json::Value::as_str)
					.unwrap_or_default()
					.to_string(),
			);
			tool_output(
				object,
				"call_id",
				calls,
				completed_outputs,
				|declared| matches!(declared, DeclaredTool::Wrapped(wrapped) if wrapped.kind == WrappedKind::ApplyPatch),
				content,
				failed,
			)?
		},
		_ => return Ok(None),
	};
	Ok(Some((role, block)))
}

fn tool_action_call(
	object: &serde_json::Map<String, serde_json::Value>,
	state: &State,
	calls: &mut HashMap<String, DeclaredTool>,
	kind: WrappedKind,
	field: &str,
) -> Result<(messages::Role, messages::ContentBlock), AIError> {
	let known = if kind == WrappedKind::Shell {
		&["type", "id", "call_id", "action", "status", "environment"][..]
	} else {
		&["type", "id", "call_id", field, "status"][..]
	};
	if has_unknown_field(object, known) {
		return Err(invalid_tool_history());
	}
	validate_terminal_call_status(object.get("status"))?;
	match kind {
		WrappedKind::LocalShell => {
			history_call_id(object, "id")?;
		},
		WrappedKind::Shell | WrappedKind::ApplyPatch => validate_optional_string(object, "id")?,
		_ => unreachable!("only action tools use this helper"),
	}
	if kind == WrappedKind::Shell
		&& object
			.get("environment")
			.is_some_and(|value| !shell_environment_is_local(value))
	{
		return Err(invalid_tool_history());
	}
	let call_id = history_call_id(object, "call_id")?;
	let value = object
		.get(field)
		.filter(|value| value.is_object())
		.cloned()
		.ok_or_else(invalid_tool_history)?;
	validate_action(&kind, &value)?;
	let upstream_name = find_wrapped_kind(state, kind.clone()).map_err(|_| invalid_tool_history())?;
	let declared = state
		.tools
		.get(&upstream_name)
		.cloned()
		.expect("wrapped name came from state");
	record_call(calls, call_id, declared)?;
	Ok((
		messages::Role::Assistant,
		messages::ContentBlock::ToolUse {
			id: call_id.to_string(),
			name: upstream_name,
			input: serde_json::json!({(field): value}),
			caller: None,
			cache_control: None,
		},
	))
}

fn find_call_declaration(
	state: &State,
	name: &str,
	namespace: Option<&str>,
	wrapped_kind: WrappedKind,
	allow_function: bool,
) -> Result<(String, DeclaredTool), AIError> {
	let (upstream, declared) = state
		.tools
		.iter()
		.filter(|(upstream, declared)| match declared {
			DeclaredTool::Function => allow_function && namespace.is_none() && upstream.as_str() == name,
			DeclaredTool::Wrapped(wrapped) => {
				wrapped.kind == wrapped_kind
					&& wrapped.name == name
					&& wrapped.namespace.as_deref() == namespace
			},
		})
		.exactly_one()
		.map_err(|_| invalid_tool_history())?;
	Ok((upstream.clone(), declared.clone()))
}

fn record_call(
	calls: &mut HashMap<String, DeclaredTool>,
	call_id: &str,
	declared: DeclaredTool,
) -> Result<(), AIError> {
	if calls.insert(call_id.to_string(), declared).is_some() {
		return Err(invalid_tool_history());
	}
	Ok(())
}

fn tool_output(
	object: &serde_json::Map<String, serde_json::Value>,
	call_id_field: &str,
	calls: &HashMap<String, DeclaredTool>,
	completed_outputs: &mut HashSet<String>,
	matches_kind: impl Fn(&DeclaredTool) -> bool,
	content: messages::ToolResultContent,
	is_error: bool,
) -> Result<(messages::Role, messages::ContentBlock), AIError> {
	let call_id = history_call_id(object, call_id_field)?;
	let declared = calls.get(call_id).ok_or_else(invalid_tool_history)?;
	if !matches_kind(declared) || !completed_outputs.insert(call_id.to_string()) {
		return Err(invalid_tool_history());
	}
	Ok((
		messages::Role::User,
		messages::ContentBlock::ToolResult {
			tool_use_id: call_id.to_string(),
			content,
			cache_control: None,
			is_error: is_error.then_some(true),
		},
	))
}

fn tool_result_content(
	value: Option<&serde_json::Value>,
) -> Result<messages::ToolResultContent, AIError> {
	match value {
		Some(serde_json::Value::String(text)) => Ok(messages::ToolResultContent::Text(text.clone())),
		Some(serde_json::Value::Array(parts)) => parts
			.iter()
			.map(tool_result_content_part)
			.collect::<Result<Vec<_>, _>>()
			.map(messages::ToolResultContent::Array),
		_ => Err(invalid_tool_history()),
	}
}

fn tool_result_content_part(
	part: &serde_json::Value,
) -> Result<messages::ToolResultContentPart, AIError> {
	let object = part.as_object().ok_or_else(invalid_tool_history)?;
	match object.get("type").and_then(serde_json::Value::as_str) {
		Some("input_text") if !has_unknown_field(object, &["type", "text"]) => object
			.get("text")
			.and_then(serde_json::Value::as_str)
			.map(|text| messages::ToolResultContentPart::Text {
				text: text.to_string(),
				citations: None,
				cache_control: None,
			})
			.ok_or_else(invalid_tool_history),
		Some("input_image") => match input_image_content(object)? {
			messages::ContentBlock::Image(image) => Ok(messages::ToolResultContentPart::Image {
				source: image.source,
				cache_control: None,
			}),
			_ => unreachable!("input image helper returns an image"),
		},
		Some("input_file") => match input_file_content(object)? {
			messages::ContentBlock::Document(document) => Ok(messages::ToolResultContentPart::Document {
				source: document.source,
				cache_control: None,
				citations: None,
				context: None,
				title: document.title,
			}),
			_ => unreachable!("input file helper returns a document"),
		},
		_ => Err(invalid_tool_history()),
	}
}

fn validate_terminal_call_status(value: Option<&serde_json::Value>) -> Result<(), AIError> {
	match value {
		None => Ok(()),
		Some(serde_json::Value::String(status)) if status == "completed" || status == "incomplete" => {
			Ok(())
		},
		_ => Err(invalid_tool_history()),
	}
}

fn function_output_status_is_error(value: Option<&serde_json::Value>) -> Result<bool, AIError> {
	match value {
		None => Ok(false),
		Some(serde_json::Value::String(status)) if status == "completed" => Ok(false),
		Some(serde_json::Value::String(status)) if status == "incomplete" => Ok(true),
		_ => Err(invalid_tool_history()),
	}
}

fn validate_action(kind: &WrappedKind, value: &serde_json::Value) -> Result<(), AIError> {
	let object = value.as_object().ok_or_else(invalid_tool_history)?;
	match kind {
		WrappedKind::LocalShell => {
			if has_unknown_field(
				object,
				&["command", "env", "timeout_ms", "user", "working_directory"],
			) || !string_array(object.get("command"))
				|| !object.get("env").is_some_and(string_map)
				|| !optional_u64(object.get("timeout_ms"))
				|| !optional_nullable_string_value(object.get("user"))
				|| !optional_nullable_string_value(object.get("working_directory"))
			{
				return Err(invalid_tool_history());
			}
		},
		WrappedKind::Shell => {
			if has_unknown_field(object, &["commands", "timeout_ms", "max_output_length"])
				|| !string_array(object.get("commands"))
				|| !optional_u64(object.get("timeout_ms"))
				|| !optional_u64(object.get("max_output_length"))
			{
				return Err(invalid_tool_history());
			}
		},
		WrappedKind::ApplyPatch => validate_patch_operation(object)?,
		_ => unreachable!("only action tools use this helper"),
	}
	Ok(())
}

fn validate_patch_operation(
	object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), AIError> {
	let required_strings = |fields: &[&str]| {
		fields.iter().all(|field| {
			object
				.get(*field)
				.and_then(serde_json::Value::as_str)
				.is_some()
		})
	};
	let valid = match object.get("type").and_then(serde_json::Value::as_str) {
		Some("create_file") | Some("update_file") => {
			!has_unknown_field(object, &["type", "path", "diff"]) && required_strings(&["path", "diff"])
		},
		Some("delete_file") => {
			!has_unknown_field(object, &["type", "path"]) && required_strings(&["path"])
		},
		_ => false,
	};
	valid.then_some(()).ok_or_else(invalid_tool_history)
}

fn shell_environment_is_local(value: &serde_json::Value) -> bool {
	let Some(object) = value.as_object() else {
		return false;
	};
	object.len() == 1 && object.get("type").and_then(serde_json::Value::as_str) == Some("local")
}

fn string_array(value: Option<&serde_json::Value>) -> bool {
	value.is_some_and(|value| {
		value
			.as_array()
			.is_some_and(|values| values.iter().all(serde_json::Value::is_string))
	})
}

fn string_map(value: &serde_json::Value) -> bool {
	value
		.as_object()
		.is_some_and(|values| values.values().all(serde_json::Value::is_string))
}

fn optional_u64(value: Option<&serde_json::Value>) -> bool {
	matches!(value, None | Some(serde_json::Value::Null))
		|| value.is_some_and(|value| value.as_u64().is_some())
}

fn optional_nullable_string_value(value: Option<&serde_json::Value>) -> bool {
	matches!(
		value,
		None | Some(serde_json::Value::Null | serde_json::Value::String(_))
	)
}

fn history_call_id<'a>(
	object: &'a serde_json::Map<String, serde_json::Value>,
	field: &str,
) -> Result<&'a str, AIError> {
	history_string(object, field).and_then(|value| {
		(!value.is_empty())
			.then_some(value)
			.ok_or_else(invalid_tool_history)
	})
}

fn history_string<'a>(
	object: &'a serde_json::Map<String, serde_json::Value>,
	field: &str,
) -> Result<&'a str, AIError> {
	object
		.get(field)
		.and_then(serde_json::Value::as_str)
		.ok_or_else(invalid_tool_history)
}

fn optional_history_string<'a>(
	object: &'a serde_json::Map<String, serde_json::Value>,
	field: &str,
) -> Result<Option<&'a str>, AIError> {
	match object.get(field) {
		None => Ok(None),
		Some(serde_json::Value::String(value)) if !value.is_empty() => Ok(Some(value)),
		_ => Err(invalid_tool_history()),
	}
}

fn validate_optional_string(
	object: &serde_json::Map<String, serde_json::Value>,
	field: &str,
) -> Result<(), AIError> {
	match object.get(field) {
		None | Some(serde_json::Value::String(_)) => Ok(()),
		_ => Err(invalid_tool_history()),
	}
}

fn invalid_tool_history() -> AIError {
	AIError::UnsupportedConversion(strng::literal!("invalid Responses tool history"))
}

fn validate_assistant(object: &serde_json::Map<String, serde_json::Value>) -> Result<(), AIError> {
	let valid_phase = match object.get("phase") {
		None => true,
		Some(serde_json::Value::String(phase)) => phase == "commentary" || phase == "final_answer",
		_ => false,
	};
	if !valid_phase {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses assistant phase"
		)));
	}
	let valid_status = match object.get("status") {
		None => true,
		Some(serde_json::Value::String(status)) => status == "completed" || status == "incomplete",
		_ => false,
	};
	if !valid_status {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses assistant status"
		)));
	}
	Ok(())
}

fn assistant_text_content(content: &serde_json::Value) -> Result<Vec<String>, AIError> {
	if matches!(content, serde_json::Value::String(text) if text.is_empty())
		|| matches!(content, serde_json::Value::Array(parts) if parts.is_empty())
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses assistant content must not be empty"
		)));
	}
	match content {
		serde_json::Value::String(text) => Ok(vec![text.clone()]),
		serde_json::Value::Array(parts) => parts
			.iter()
			.map(|part| {
				let object = part.as_object().ok_or_else(|| {
					AIError::UnsupportedConversion(strng::literal!("unsupported Responses assistant content"))
				})?;
				match object.get("type").and_then(serde_json::Value::as_str) {
					Some("output_text") => {
						if has_unknown_field(object, &["type", "text", "annotations", "logprobs"]) {
							return Err(AIError::UnsupportedConversion(strng::literal!(
								"unsupported Responses assistant content field"
							)));
						}
						let valid_annotations = match object.get("annotations") {
							None => true,
							Some(serde_json::Value::Array(values)) => values.is_empty(),
							_ => false,
						};
						if !valid_annotations {
							return Err(AIError::UnsupportedConversion(strng::literal!(
								"Responses assistant annotations are unsupported"
							)));
						}
						let valid_logprobs = match object.get("logprobs") {
							None | Some(serde_json::Value::Null) => true,
							Some(serde_json::Value::Array(values)) => values.is_empty(),
							_ => false,
						};
						if !valid_logprobs {
							return Err(AIError::UnsupportedConversion(strng::literal!(
								"Responses assistant logprobs are unsupported"
							)));
						}
						object
							.get("text")
							.and_then(serde_json::Value::as_str)
							.map(ToOwned::to_owned)
							.ok_or_else(|| {
								AIError::UnsupportedConversion(strng::literal!(
									"Responses assistant text is required"
								))
							})
					},
					// A prior refusal (this crate's own output shape, see `response_output`) must be
					// replayable as history: this route requires `store:false`, so the client resends
					// the exact assistant content the API returned, refusals included.
					Some("refusal") => {
						if has_unknown_field(object, &["type", "refusal"]) {
							return Err(AIError::UnsupportedConversion(strng::literal!(
								"unsupported Responses assistant content field"
							)));
						}
						let refusal = object
							.get("refusal")
							.and_then(serde_json::Value::as_str)
							.ok_or_else(|| {
								AIError::UnsupportedConversion(strng::literal!(
									"Responses assistant refusal text is required"
								))
							})?;
						if refusal.is_empty() {
							return Err(AIError::UnsupportedConversion(strng::literal!(
								"a content-free Responses refusal cannot be replayed as history"
							)));
						}
						Ok(refusal.to_owned())
					},
					_ => Err(AIError::UnsupportedConversion(strng::literal!(
						"unsupported Responses assistant content"
					))),
				}
			})
			.collect(),
		_ => Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses assistant content"
		))),
	}
}

fn user_content(content: &serde_json::Value) -> Result<Vec<messages::ContentBlock>, AIError> {
	if matches!(content, serde_json::Value::String(text) if text.is_empty())
		|| matches!(content, serde_json::Value::Array(parts) if parts.is_empty())
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses message content must not be empty"
		)));
	}
	match content {
		serde_json::Value::String(text) => Ok(vec![text_block(text.clone())]),
		serde_json::Value::Array(parts) => parts
			.iter()
			.map(|part| {
				let object = part.as_object().ok_or_else(|| {
					AIError::UnsupportedConversion(strng::literal!("unsupported Responses message content"))
				})?;
				match object.get("type").and_then(serde_json::Value::as_str) {
					Some("input_text") => {
						if has_unknown_field(object, &["type", "text"]) {
							return Err(AIError::UnsupportedConversion(strng::literal!(
								"unsupported Responses message content field"
							)));
						}
						object
							.get("text")
							.and_then(serde_json::Value::as_str)
							.map(|text| text_block(text.to_string()))
							.ok_or_else(|| {
								AIError::UnsupportedConversion(strng::literal!(
									"Responses text content is required"
								))
							})
					},
					Some("input_image") => input_image_content(object),
					Some("input_file") => input_file_content(object),
					_ => Err(AIError::UnsupportedConversion(strng::literal!(
						"unsupported Responses message content"
					))),
				}
			})
			.collect(),
		_ => Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses message content"
		))),
	}
}

fn input_image_content(
	object: &serde_json::Map<String, serde_json::Value>,
) -> Result<messages::ContentBlock, AIError> {
	let valid_detail = match object.get("detail") {
		None => true,
		Some(serde_json::Value::String(detail)) => detail == "auto",
		Some(_) => false,
	};
	if has_unknown_field(object, &["type", "image_url", "file_id", "detail"])
		|| object.contains_key("file_id")
		|| !valid_detail
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses input image"
		)));
	}
	let url = object
		.get("image_url")
		.and_then(serde_json::Value::as_str)
		.ok_or_else(|| {
			AIError::UnsupportedConversion(strng::literal!("Responses input image URL is required"))
		})?;
	let source = if let Some((media_type, data)) = parse_base64_data_url(url) {
		if !matches!(
			media_type,
			"image/jpeg" | "image/png" | "image/gif" | "image/webp"
		) {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"unsupported Responses input image media type"
			)));
		}
		serde_json::json!({"type": "base64", "media_type": media_type, "data": data})
	} else {
		if !is_absolute_http_url(url) {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"invalid Responses input image URL"
			)));
		}
		serde_json::json!({"type": "url", "url": url})
	};
	Ok(messages::ContentBlock::Image(messages::ContentImageBlock {
		source,
		cache_control: None,
	}))
}

fn parse_base64_data_url(url: &str) -> Option<(&str, &str)> {
	let (media_type, data) = crate::conversion::completions::parse_data_url(url)?;
	base64::engine::general_purpose::STANDARD
		.decode(data)
		.ok()?;
	Some((media_type, data))
}

fn is_absolute_http_url(url: &str) -> bool {
	let Ok(uri) = url.parse::<http::Uri>() else {
		return false;
	};
	matches!(uri.scheme_str(), Some("http" | "https"))
		&& uri.authority().is_some()
		&& uri.host().is_some_and(|host| !host.is_empty())
}

fn input_file_content(
	object: &serde_json::Map<String, serde_json::Value>,
) -> Result<messages::ContentBlock, AIError> {
	if has_unknown_field(
		object,
		&[
			"type",
			"file_data",
			"file_url",
			"file_id",
			"filename",
			"detail",
		],
	) || object.contains_key("file_id")
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses input file"
		)));
	}
	let valid_detail = match object.get("detail") {
		None => true,
		Some(serde_json::Value::String(detail)) => matches!(detail.as_str(), "auto" | "low"),
		Some(_) => false,
	};
	if !valid_detail {
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses input file detail"
		)));
	}
	let title = match object.get("filename") {
		None => None,
		Some(serde_json::Value::String(filename)) if filename.is_empty() => None,
		Some(serde_json::Value::String(filename)) => Some(filename.clone()),
		Some(_) => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"unsupported Responses input file filename"
			)));
		},
	};
	let source = match (object.get("file_data"), object.get("file_url")) {
		(Some(serde_json::Value::String(file_data)), None) => file_data_source(file_data)?,
		(None, Some(serde_json::Value::String(file_url))) => {
			if !is_absolute_http_url(file_url) {
				return Err(AIError::UnsupportedConversion(strng::literal!(
					"invalid Responses input file URL"
				)));
			}
			serde_json::json!({"type": "url", "url": file_url})
		},
		_ => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"Responses input file requires exactly one source"
			)));
		},
	};
	Ok(messages::ContentBlock::Document(
		messages::ContentDocumentBlock {
			source,
			cache_control: None,
			citations: None,
			context: None,
			title,
		},
	))
}

fn file_data_source(file_data: &str) -> Result<serde_json::Value, AIError> {
	let (media_type, data) = parse_base64_data_url(file_data).ok_or_else(|| {
		AIError::UnsupportedConversion(strng::literal!("invalid Responses input file data"))
	})?;
	let source = match media_type {
		"application/pdf" => serde_json::json!({
			"type": "base64",
			"media_type": media_type,
			"data": data
		}),
		"text/plain" => {
			let bytes = base64::engine::general_purpose::STANDARD
				.decode(data)
				.map_err(|_| {
					AIError::UnsupportedConversion(strng::literal!("invalid Responses input file data"))
				})?;
			let text = String::from_utf8(bytes).map_err(|_| {
				AIError::UnsupportedConversion(strng::literal!("Responses text file must be UTF-8"))
			})?;
			serde_json::json!({
				"type": "text",
				"media_type": media_type,
				"data": text
			})
		},
		_ => {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"unsupported Responses input file media type"
			)));
		},
	};
	Ok(source)
}

fn text_content(content: &serde_json::Value, expected_type: &str) -> Result<Vec<String>, AIError> {
	if matches!(content, serde_json::Value::String(text) if text.is_empty())
		|| matches!(content, serde_json::Value::Array(parts) if parts.is_empty())
	{
		return Err(AIError::UnsupportedConversion(strng::literal!(
			"Responses message content must not be empty"
		)));
	}
	match content {
		serde_json::Value::String(text) => Ok(vec![text.clone()]),
		serde_json::Value::Array(parts) => parts
			.iter()
			.map(|part| {
				let object = part.as_object().ok_or_else(|| {
					AIError::UnsupportedConversion(strng::literal!("unsupported Responses message content"))
				})?;
				if object.get("type").and_then(serde_json::Value::as_str) != Some(expected_type) {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"unsupported Responses message content"
					)));
				}
				if has_unknown_field(object, &["type", "text"]) {
					return Err(AIError::UnsupportedConversion(strng::literal!(
						"unsupported Responses message content field"
					)));
				}
				object
					.get("text")
					.and_then(serde_json::Value::as_str)
					.map(ToOwned::to_owned)
					.ok_or_else(|| {
						AIError::UnsupportedConversion(strng::literal!("Responses text content is required"))
					})
			})
			.collect(),
		_ => Err(AIError::UnsupportedConversion(strng::literal!(
			"unsupported Responses message content"
		))),
	}
}

fn push_message(output: &mut Vec<messages::Message>, role: messages::Role, texts: Vec<String>) {
	let content = texts.into_iter().map(text_block).collect::<Vec<_>>();
	push_blocks(output, role, content);
}

fn text_block(text: String) -> messages::ContentBlock {
	messages::ContentBlock::Text(messages::ContentTextBlock {
		text,
		citations: None,
		cache_control: None,
	})
}

fn push_blocks(
	output: &mut Vec<messages::Message>,
	role: messages::Role,
	content: Vec<messages::ContentBlock>,
) {
	if let Some(last) = output.last_mut()
		&& last.role == role
	{
		last.content.extend(content);
	} else {
		output.push(messages::Message { role, content });
	}
}

#[cfg(test)]
mod tests;
