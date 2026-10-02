//! Server-side web search for the OpenAI Chat Completions API.
//!
//! Chat Completions has no server-executed entries in its `tools` array; its one server-side
//! feature is the `web_search_options` request field, which asks the provider to search the web.
//! A provider that does not implement it ignores the field. The helpers here let the gateway
//! present that request to the model as an ordinary function tool, recognise the resulting tool
//! call, and continue the same turn with the tool output before returning one finished completion
//! to the client.
//!
//! Everything in this module is plain data manipulation. Executing the tool and re-sending the
//! request belong to the caller.

use std::collections::{BTreeMap, HashSet};

use bytes::Bytes;
use serde_json::{Map, Value, json};

use crate::server_tools::{
	InterceptedTool, SseEvent, ToolDefinition, ToolUse, TypeMatch, fill_total, function_definition,
	push_str,
};
use crate::types::completions::{Request, RequestMessage};

#[cfg(test)]
#[path = "completions_tools_tests.rs"]
mod tests;

/// The tool type a `web_search_options` field declares, matched against the operator mappings.
pub const WEB_SEARCH_OPTIONS_TYPE: &str = "web_search_options";
/// The function name the model calls in place of `web_search_options`.
pub const WEB_SEARCH_TOOL_NAME: &str = "web_search";

/// The names of the client's own function tools.
pub fn function_names(req: &Request) -> HashSet<String> {
	req
		.tools
		.iter()
		.flatten()
		.filter_map(|tool| {
			tool
				.get("function")
				.and_then(|f| f.get("name"))
				.and_then(Value::as_str)
				.map(str::to_string)
		})
		.collect()
}

/// The server-side search the client asked for, when an operator mapping covers it and the
/// client has no function tool of the same name.
pub fn find_web_search(req: &Request, matchers: &[TypeMatch]) -> Option<InterceptedTool> {
	req.rest.get("web_search_options")?;
	let mapping = matchers
		.iter()
		.position(|m| m.matches(WEB_SEARCH_OPTIONS_TYPE))?;
	if function_names(req).contains(WEB_SEARCH_TOOL_NAME) {
		return None;
	}
	Some(InterceptedTool {
		name: WEB_SEARCH_TOOL_NAME.to_string(),
		tool_type: WEB_SEARCH_OPTIONS_TYPE.to_string(),
		mapping,
		max_uses: None,
	})
}

/// The client's `web_search_options`, as sent.
pub fn declared_web_search_options(req: &Request) -> Value {
	req
		.rest
		.get("web_search_options")
		.cloned()
		.unwrap_or(Value::Null)
}

/// Replace the `web_search_options` field with a function tool the model can call.
pub fn rewrite_web_search(req: &mut Request, tool: &InterceptedTool, def: &ToolDefinition) {
	if let Some(rest) = req.rest.as_object_mut() {
		rest.remove("web_search_options");
	}
	req
		.tools
		.get_or_insert_with(Vec::new)
		.push(json!({"type": "function", "function": function_definition(&tool.name, def)}));
}

/// Remove the named function tools, and a `tool_choice` that forces one of them, so the model has
/// to answer without them.
pub fn remove_function_tools(req: &mut Request, names: &HashSet<&str>) {
	if let Some(list) = req.tools.as_mut() {
		list.retain(|tool| {
			!tool
				.get("function")
				.and_then(|f| f.get("name"))
				.and_then(Value::as_str)
				.is_some_and(|n| names.contains(n))
		});
	}
	let forced = req
		.tool_choice
		.as_ref()
		.and_then(|c| c.get("function"))
		.and_then(|f| f.get("name"))
		.and_then(Value::as_str)
		.is_some_and(|n| names.contains(n));
	if forced {
		req.tool_choice = None;
	}
}

fn first_choice(message: &Value) -> Option<&Value> {
	message.get("choices")?.as_array()?.first()
}

fn first_choice_mut(message: &mut Value) -> Option<&mut Value> {
	message.get_mut("choices")?.as_array_mut()?.first_mut()
}

/// The tool calls of the first choice, in order.
pub fn tool_calls(message: &Value) -> Vec<ToolUse> {
	first_choice(message)
		.and_then(|c| c.get("message"))
		.and_then(|m| m.get("tool_calls"))
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|call| {
			let function = call.get("function")?;
			let arguments = function
				.get("arguments")
				.and_then(Value::as_str)
				.unwrap_or("");
			let input = if arguments.trim().is_empty() {
				Value::Object(Map::new())
			} else {
				serde_json::from_str(arguments).unwrap_or_else(|_| json!({ "_raw": arguments }))
			};
			Some(ToolUse {
				id: call.get("id")?.as_str()?.to_string(),
				name: function.get("name")?.as_str()?.to_string(),
				input,
			})
		})
		.collect()
}

/// Remove the named tool calls from the first choice. When none remain, the choice ends the turn
/// normally.
pub fn strip_tool_calls(message: &mut Value, names: &HashSet<&str>) -> usize {
	let Some(choice) = first_choice_mut(message) else {
		return 0;
	};
	let removed = {
		let Some(calls) = choice
			.get_mut("message")
			.and_then(|m| m.get_mut("tool_calls"))
			.and_then(Value::as_array_mut)
		else {
			return 0;
		};
		let before = calls.len();
		calls.retain(|call| {
			!call
				.get("function")
				.and_then(|f| f.get("name"))
				.and_then(Value::as_str)
				.is_some_and(|n| names.contains(n))
		});
		before - calls.len()
	};
	let none_left = choice["message"]["tool_calls"]
		.as_array()
		.is_some_and(Vec::is_empty);
	if none_left {
		if let Some(msg) = choice.get_mut("message").and_then(Value::as_object_mut) {
			msg.remove("tool_calls");
		}
		if choice.get("finish_reason").and_then(Value::as_str) == Some("tool_calls") {
			choice["finish_reason"] = json!("stop");
		}
	}
	removed
}

/// Append the model's message and the tool messages to the conversation, ready to be sent again.
pub fn append_tool_turn(req: &mut Request, completion: &Value, tool_messages: Vec<Value>) {
	let assistant = first_choice(completion)
		.and_then(|c| c.get("message"))
		.cloned()
		.unwrap_or(Value::Null);
	let assistant =
		serde_json::from_value::<RequestMessage>(assistant).unwrap_or_else(|_| RequestMessage {
			role: "assistant".to_string(),
			name: None,
			content: None,
			tool_call_id: None,
			tool_calls: None,
			rest: Default::default(),
		});
	req.messages.push(assistant);
	for message in tool_messages {
		if let Ok(message) = serde_json::from_value::<RequestMessage>(message) {
			req.messages.push(message);
		}
	}
}

/// Build the `tool` role message that carries a tool output.
pub fn tool_message(call_id: &str, content: String) -> Value {
	json!({"role": "tool", "tool_call_id": call_id, "content": content})
}

/// Overwrite the usage of a completion with the summed totals.
pub fn set_usage(message: &mut Value, totals: &Value) {
	crate::server_tools::set_usage(message, totals);
	if let Some(usage) = message.get_mut("usage") {
		fill_total(usage, ["prompt_tokens", "completion_tokens"]);
	}
}

const DONE: &str = "[DONE]";

/// Rebuilds a complete chat completion from its streamed chunks.
#[derive(Debug, Default)]
pub struct ChunkAccumulator {
	base: Option<Map<String, Value>>,
	role: Option<String>,
	content: Option<String>,
	refusal: Option<String>,
	tool_calls: BTreeMap<u64, Value>,
	finish_reason: Option<Value>,
	usage: Option<Value>,
	done: bool,
}

impl ChunkAccumulator {
	pub fn feed(&mut self, ev: &SseEvent) {
		if ev.data.trim() == DONE {
			self.done = true;
			return;
		}
		let Ok(Value::Object(mut chunk)) = serde_json::from_str::<Value>(&ev.data) else {
			return;
		};
		if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
			self.usage = Some(usage.clone());
		}
		let choices = chunk.remove("choices");
		if self.base.is_none() {
			chunk.remove("usage");
			chunk.insert("object".to_string(), json!("chat.completion"));
			self.base = Some(chunk);
		}
		let Some(choice) = choices
			.as_ref()
			.and_then(Value::as_array)
			.and_then(|c| c.first())
		else {
			return;
		};
		if let Some(reason) = choice.get("finish_reason").filter(|r| !r.is_null()) {
			self.finish_reason = Some(reason.clone());
		}
		let Some(delta) = choice.get("delta") else {
			return;
		};
		if let Some(role) = delta.get("role").and_then(Value::as_str) {
			self.role = Some(role.to_string());
		}
		if let Some(text) = delta.get("content").and_then(Value::as_str) {
			self.content.get_or_insert_with(String::new).push_str(text);
		}
		if let Some(text) = delta.get("refusal").and_then(Value::as_str) {
			self.refusal.get_or_insert_with(String::new).push_str(text);
		}
		for call in delta
			.get("tool_calls")
			.and_then(Value::as_array)
			.into_iter()
			.flatten()
		{
			let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
			let entry = self.tool_calls.entry(index).or_insert_with(
				|| json!({"id": "", "type": "function", "function": {"name": "", "arguments": ""}}),
			);
			if let Some(id) = call.get("id").and_then(Value::as_str) {
				entry["id"] = json!(id);
			}
			if let Some(function) = call.get("function") {
				if let Some(name) = function.get("name").and_then(Value::as_str) {
					entry["function"]["name"] = json!(name);
				}
				if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
					push_str(&mut entry["function"]["arguments"], arguments);
				}
			}
		}
	}

	pub fn feed_all(&mut self, events: &[SseEvent]) {
		for ev in events {
			self.feed(ev);
		}
	}

	/// True once `[DONE]` or a finish reason was seen.
	pub fn is_complete(&self) -> bool {
		self.done || self.finish_reason.is_some()
	}

	/// The rebuilt completion, or `None` when no chunk was seen.
	pub fn finish(self) -> Option<Value> {
		let mut response = self.base?;
		let mut message = json!({
			"role": self.role.unwrap_or_else(|| "assistant".to_string()),
			"content": self.content,
		});
		if let Some(refusal) = self.refusal {
			message["refusal"] = json!(refusal);
		}
		if !self.tool_calls.is_empty() {
			message["tool_calls"] = Value::Array(self.tool_calls.into_values().collect());
		}
		response.insert(
			"choices".to_string(),
			json!([{
				"index": 0,
				"message": message,
				"finish_reason": self.finish_reason.unwrap_or_else(|| json!("stop")),
			}]),
		);
		if let Some(usage) = self.usage {
			response.insert("usage".to_string(), usage);
		}
		Some(Value::Object(response))
	}
}

fn chunk(base: &Map<String, Value>, choices: Value, usage: Option<Value>) -> SseEvent {
	let mut data = base.clone();
	data.insert("object".to_string(), json!("chat.completion.chunk"));
	data.remove("usage");
	data.insert("choices".to_string(), choices);
	if let Some(usage) = usage {
		data.insert("usage".to_string(), usage);
	}
	SseEvent {
		event: None,
		data: Value::Object(data).to_string(),
	}
}

fn base_of(message: &Value) -> Map<String, Value> {
	let mut base = Map::new();
	for key in [
		"id",
		"created",
		"model",
		"system_fingerprint",
		"service_tier",
	] {
		if let Some(v) = message.get(key) {
			base.insert(key.to_string(), v.clone());
		}
	}
	base
}

/// Render a complete completion as the chunk sequence a streaming client expects.
pub fn synthesize_sse(message: &Value) -> Vec<SseEvent> {
	let base = base_of(message);
	let choice = first_choice(message).cloned().unwrap_or(Value::Null);
	let assistant = choice.get("message").cloned().unwrap_or(Value::Null);
	let delta =
		|delta: Value, finish: Value| json!([{"index": 0, "delta": delta, "finish_reason": finish}]);
	let mut events = vec![chunk(
		&base,
		delta(json!({"role": "assistant", "content": ""}), Value::Null),
		None,
	)];
	if let Some(text) = assistant.get("content").and_then(Value::as_str)
		&& !text.is_empty()
	{
		events.push(chunk(
			&base,
			delta(json!({"content": text}), Value::Null),
			None,
		));
	}
	if let Some(refusal) = assistant.get("refusal").and_then(Value::as_str)
		&& !refusal.is_empty()
	{
		events.push(chunk(
			&base,
			delta(json!({"refusal": refusal}), Value::Null),
			None,
		));
	}
	for (index, call) in assistant
		.get("tool_calls")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.enumerate()
	{
		let mut streamed = call.clone();
		streamed["index"] = json!(index);
		events.push(chunk(
			&base,
			delta(json!({"tool_calls": [streamed]}), Value::Null),
			None,
		));
	}
	events.push(chunk(
		&base,
		delta(
			json!({}),
			choice
				.get("finish_reason")
				.cloned()
				.unwrap_or_else(|| json!("stop")),
		),
		None,
	));
	if let Some(usage) = message.get("usage").filter(|u| u.is_object()) {
		events.push(chunk(&base, json!([]), Some(usage.clone())));
	}
	events.push(SseEvent {
		event: None,
		data: DONE.to_string(),
	});
	events
}

/// Rewrite the usage carried by the stream to the summed totals, adding a usage chunk before
/// `[DONE]` when the stream had none.
pub fn patch_usage(events: &mut Vec<SseEvent>, totals: &Value) {
	let mut patched = false;
	let mut base: Option<Map<String, Value>> = None;
	for ev in events.iter_mut() {
		let Ok(mut data) = serde_json::from_str::<Value>(&ev.data) else {
			continue;
		};
		if base.is_none() && data.is_object() {
			base = Some(base_of(&data));
		}
		if data.get("usage").is_some_and(Value::is_object) {
			set_usage(&mut data, totals);
			ev.data = data.to_string();
			patched = true;
		}
	}
	if !patched {
		let mut usage = json!({"usage": {}});
		set_usage(&mut usage, totals);
		let extra = chunk(
			&base.unwrap_or_default(),
			json!([]),
			Some(usage["usage"].take()),
		);
		let at = events
			.iter()
			.position(|ev| ev.data.trim() == DONE)
			.unwrap_or(events.len());
		events.insert(at, extra);
	}
}

/// An error object in the shape OpenAI streams use, followed by `[DONE]`.
pub fn error_event(message: &str) -> Bytes {
	let error = json!({
		"error": {"message": message, "type": "server_error", "code": "server_tool_error"},
	});
	Bytes::from(format!("data: {error}\n\ndata: {DONE}\n\n"))
}
