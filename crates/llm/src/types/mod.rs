pub mod bedrock;
pub mod completions;
pub mod count_tokens;
pub mod detect;
pub mod embeddings;
pub mod gemini;
pub mod messages;
pub mod rerank;
pub mod responses;
pub mod vertex;
pub mod vertex_gemini;

use agent_core::prelude::Strng;
use agent_core::strng;
use serde::Serialize;

use crate::{AIError, LLMRequest, LLMResponse, apply};

pub enum ChatRequest {
	Completions(completions::Request),
	Messages(messages::Request),
	Responses(responses::Request),
	/// Native Gemini generateContent/streamGenerateContent body
	Gemini(gemini::GenerateContentRequest),
}

pub(crate) fn thinking_budget_for_reasoning_effort(
	effort: &completions::typed::ReasoningEffort,
) -> Option<u64> {
	use completions::typed::ReasoningEffort;

	match effort {
		ReasoningEffort::None => None,
		ReasoningEffort::Minimal | ReasoningEffort::Low => Some(1024),
		ReasoningEffort::Medium => Some(2048),
		ReasoningEffort::High => Some(4096),
		ReasoningEffort::Xhigh => Some(8192),
		ReasoningEffort::Max => Some(16384),
	}
}

pub(crate) fn anthropic_effort_for_reasoning_effort(
	effort: &completions::typed::ReasoningEffort,
) -> Option<messages::typed::ThinkingEffort> {
	use completions::typed::ReasoningEffort;
	use messages::typed::ThinkingEffort;

	match effort {
		ReasoningEffort::None => None,
		ReasoningEffort::Minimal | ReasoningEffort::Low => Some(ThinkingEffort::Low),
		ReasoningEffort::Medium => Some(ThinkingEffort::Medium),
		ReasoningEffort::High => Some(ThinkingEffort::High),
		ReasoningEffort::Xhigh => Some(ThinkingEffort::Xhigh),
		ReasoningEffort::Max => Some(ThinkingEffort::Max),
	}
}

pub(crate) fn thinking_budget_for_anthropic_effort(effort: messages::typed::ThinkingEffort) -> u64 {
	use messages::typed::ThinkingEffort;

	match effort {
		ThinkingEffort::Low => 1024,
		ThinkingEffort::Medium => 2048,
		ThinkingEffort::High => 4096,
		ThinkingEffort::Xhigh => 8192,
		ThinkingEffort::Max => 16384,
	}
}

/// ResponseType is an abstraction over provider/endpoint specific response formats that enables
/// uniform policy enforcement and observability
pub trait ResponseType: Send + Sync {
	fn to_llm_response(&self, log_content: crate::LogContentFields) -> LLMResponse;
	fn to_webhook_choices(&self) -> Vec<crate::webhook::ResponseChoice>;
	fn set_webhook_choices(
		&mut self,
		resp: Vec<crate::webhook::ResponseChoice>,
	) -> anyhow::Result<()>;
	fn serialize(&self) -> serde_json::Result<Vec<u8>>;
	fn visit_text_mut(&mut self, f: &mut dyn FnMut(&mut String));
}

/// A category of request content that a prompt guard can inspect.
#[apply(schema_enum!)]
pub enum ContentScope {
	/// The system/developer prompt.
	SystemPrompt,
	/// Regular user/assistant message text.
	Messages,
	/// Tool call results.
	ToolOutput,
	/// Tool call arguments.
	///
	/// In APIs that send tool arguments as opaque JSON, such as Completions, the arguments are masked as a single string,
	/// meaning a prompt guard has the potential to rewrite the arguments into invalid JSON.
	ToolInput,
}

/// Recursively visit every string value in a JSON tree (tool inputs are arbitrary JSON).
pub(crate) fn visit_json_strings(value: &mut serde_json::Value, f: &mut dyn FnMut(&mut String)) {
	match value {
		serde_json::Value::String(text) => f(text),
		serde_json::Value::Array(items) => {
			for item in items {
				visit_json_strings(item, f);
			}
		},
		serde_json::Value::Object(map) => {
			for (_, item) in map.iter_mut() {
				visit_json_strings(item, f);
			}
		},
		// TODO scan numbers and bools?
		_ => {},
	}
}

/// Recursively every string value in the JSON tree at `path`; a bare string is visited as one
/// opaque value.
/// TODO Numbers and bools are not scanned.
pub(crate) fn visit_json_at(
	value: &mut serde_json::Value,
	path: &[&str],
	scope: ContentScope,
	f: &mut dyn FnMut(ContentScope, &mut String),
) {
	if let Some(target) = path.iter().try_fold(value, |v, k| v.get_mut(*k)) {
		visit_json_strings(target, &mut |text| f(scope, text));
	}
}

/// RequestType is an abstraction over provider/endpoint specific request formats that enables
/// uniform policy enforcement and observability
pub trait RequestType: Send + Sync {
	fn supports_model(&self) -> bool {
		true
	}
	fn body_is_json(&self) -> bool;
	fn model(&mut self) -> &mut Option<String>;
	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>);
	fn append_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>);
	fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError>;
	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage>;
	fn get_messages_v2(&self) -> Vec<NormalizedMessage> {
		self
			.get_messages()
			.into_iter()
			.map(NormalizedMessage::from)
			.collect()
	}
	fn set_messages(&mut self, messages: Vec<SimpleChatCompletionMessage>);
	fn to_value(&self) -> serde_json::Result<serde_json::Value>;
	fn visit_text_mut(&mut self, f: &mut dyn FnMut(ContentScope, &mut String));
}

/// Scan runs of consecutive text parts as one `sep`-joined string: `[t1, t2, img, t3]` scans
/// `"t1{sep}t2"` then `"t3"`. An edited run collapses into its last part; untouched runs pass
/// through unchanged. `preserved_rest_keys`: when a text run is masked, which keys in `rest`
/// should be preserved.
pub(crate) fn scan_text_runs<T>(
	parts: &mut Vec<T>,
	sep: &str,
	mut text_of: impl FnMut(&mut T) -> Option<&mut String>,
	mut rest_of: impl FnMut(&mut T) -> Option<&mut serde_json::Value>,
	preserved_rest_keys: &[&str],
	f: &mut dyn FnMut(&mut String),
) {
	if let [part] = parts.as_mut_slice() {
		if let Some(text) = text_of(part) {
			f(text);
		}
		return;
	}

	let mut i = 0;
	while i < parts.len() {
		let mut joined = String::new();
		let mut end = i;

		// join until we hit non-text or the end of the list
		while let Some(text) = parts.get_mut(end).and_then(&mut text_of) {
			if end > i {
				joined.push_str(sep);
			}
			joined.push_str(text);
			end += 1;
		}
		if end == i {
			i += 1;
			continue;
		}

		// don't collapse the run into a single part if `f` wouldn't mutate it
		let original = joined.clone();
		f(&mut joined);
		if joined == original {
			i = end;
			continue;
		}

		// a preserved key anywhere in the run must survive the collapse: the survivor's own
		// value wins, else carry the latest drained one; JSON null counts as absent
		for &key in preserved_rest_keys {
			let survivor_has = rest_of(&mut parts[end - 1])
				.and_then(|rest| rest.get(key))
				.is_some_and(|v| !v.is_null());
			if survivor_has {
				continue;
			}
			let carried = parts[i..end - 1].iter_mut().rev().find_map(|p| {
				rest_of(p)
					.and_then(serde_json::Value::as_object_mut)
					.and_then(|obj| obj.remove(key))
					.filter(|v| !v.is_null())
			});
			if let Some(value) = carried
				&& let Some(rest) = rest_of(&mut parts[end - 1])
			{
				// typed parts default `rest` to Null
				if !rest.is_object() {
					*rest = serde_json::Value::Object(Default::default());
				}
				if let Some(obj) = rest.as_object_mut() {
					obj.insert(key.to_string(), value);
				}
			}
		}

		// collapse the run's text into the last part, and remove the others
		if let Some(text) = text_of(&mut parts[end - 1]) {
			*text = joined;
		}
		parts.drain(i..end - 1);
		i += 1;
	}
}

/// Join text parts with `sep`, ignoring empty parts.
pub(crate) fn join_text<'a>(parts: impl IntoIterator<Item = &'a str>, sep: char) -> Strng {
	let s = parts.into_iter().fold(String::new(), |mut acc, s| {
		if !acc.is_empty() {
			acc.push(sep);
		}
		acc.push_str(s);
		acc
	});
	strng::new(&s)
}

/// SimpleChatCompletionMessage is a simplified chat message
#[apply(schema!)]
#[derive(Eq, PartialEq, cel::DynamicType)]
pub struct SimpleChatCompletionMessage {
	/// Message role, such as "system", "user", or "assistant".
	pub role: Strng,
	/// Message text content.
	pub content: Strng,
}

/// A provider-neutral request message that preserves the ordering of text, tool calls, tool
/// results, and reasoning. This is an observability representation, not a lossless wire format.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedMessage {
	/// Message role, such as "system", "user", "assistant", or "tool".
	pub role: Strng,
	pub parts: Vec<NormalizedMessagePart>,
}

impl From<SimpleChatCompletionMessage> for NormalizedMessage {
	fn from(message: SimpleChatCompletionMessage) -> Self {
		Self {
			role: message.role,
			parts: vec![NormalizedMessagePart::text(message.content)],
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NormalizedMessagePartType {
	Text,
	ToolCall,
	ToolResult,
	Reasoning,
}

/// One ordered part of a [`NormalizedMessage`]. Fields are populated according to `type`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedMessagePart {
	pub r#type: NormalizedMessagePartType,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub text: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub id: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub arguments: Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content: Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub is_error: Option<bool>,
}

impl NormalizedMessagePart {
	pub fn text(text: Strng) -> Self {
		Self {
			r#type: NormalizedMessagePartType::Text,
			text: Some(text),
			id: None,
			name: None,
			arguments: None,
			content: None,
			is_error: None,
		}
	}

	pub fn tool_call(id: Strng, name: Strng, arguments: serde_json::Value) -> Self {
		Self {
			r#type: NormalizedMessagePartType::ToolCall,
			text: None,
			id: Some(id),
			name: Some(name),
			arguments: Some(arguments),
			content: None,
			is_error: None,
		}
	}

	pub fn tool_result(
		id: Option<Strng>,
		name: Option<Strng>,
		content: serde_json::Value,
		is_error: Option<bool>,
	) -> Self {
		Self {
			r#type: NormalizedMessagePartType::ToolResult,
			text: None,
			id,
			name,
			arguments: None,
			content: Some(content),
			is_error,
		}
	}

	pub fn reasoning(content: serde_json::Value) -> Self {
		Self {
			r#type: NormalizedMessagePartType::Reasoning,
			text: None,
			id: None,
			name: None,
			arguments: None,
			content: Some(content),
			is_error: None,
		}
	}
}

pub(crate) fn normalized_tool_call(value: &serde_json::Value) -> Option<NormalizedMessagePart> {
	let function = value.get("function").unwrap_or(value);
	let name = function.get("name")?.as_str()?;
	let id = value
		.get("id")
		.or_else(|| value.get("call_id"))
		.and_then(serde_json::Value::as_str)
		.unwrap_or(name);
	let arguments = function
		.get("arguments")
		.or_else(|| function.get("input"))
		.cloned()
		.unwrap_or_else(|| serde_json::Value::Object(Default::default()));
	Some(NormalizedMessagePart::tool_call(
		strng::new(id),
		strng::new(name),
		parse_json_string(arguments),
	))
}

pub(crate) fn parse_json_string(value: serde_json::Value) -> serde_json::Value {
	match value {
		serde_json::Value::String(value) => {
			serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
		},
		value => value,
	}
}

pub(crate) fn attach_tool_result_names(messages: &mut [NormalizedMessage]) {
	let calls = messages
		.iter()
		.flat_map(|message| &message.parts)
		.filter(|part| part.r#type == NormalizedMessagePartType::ToolCall)
		.filter_map(|part| Some((part.id.as_ref()?.clone(), part.name.as_ref()?.clone())))
		.collect::<std::collections::HashMap<_, _>>();
	for part in messages.iter_mut().flat_map(|message| &mut message.parts) {
		if part.r#type == NormalizedMessagePartType::ToolResult
			&& part.name.is_none()
			&& let Some(id) = &part.id
		{
			part.name = calls.get(id).cloned();
		}
	}
}

/// ToolCall represents a single tool/function invocation surfaced for observability.
#[apply(schema!)]
#[derive(cel::DynamicType)]
pub struct ToolCall {
	pub id: Strng,
	pub name: Strng,
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::Value"))]
	pub arguments: serde_json::Value,
}

/// A single content part within an output message, per the GenAI semantic conventions.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputMessagePart {
	ToolCall {
		id: Strng,
		name: Strng,
		arguments: serde_json::Value,
	},
}

/// A structured output message for the `gen_ai.output.messages` semantic convention attribute.
#[derive(Debug, Clone, Serialize)]
pub struct OutputMessage {
	pub role: Strng,
	pub content: Vec<OutputMessagePart>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub finish_reason: Option<Strng>,
}

impl OutputMessage {
	pub fn tool_calls(&self) -> Vec<ToolCall> {
		self
			.content
			.iter()
			.map(|p| match p {
				OutputMessagePart::ToolCall {
					id,
					name,
					arguments,
				} => ToolCall {
					id: id.clone(),
					name: name.clone(),
					arguments: arguments.clone(),
				},
			})
			.collect()
	}
}

pub fn serialize_str<T: Serialize>(value: &T) -> Option<Strng> {
	serde_json::to_value(value).ok()?.as_str().map(Into::into)
}
