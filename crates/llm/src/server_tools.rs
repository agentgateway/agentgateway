//! Server-side tool fulfilment for the Anthropic Messages API.
//!
//! Anthropic-SDK clients can declare tools the *server* executes, for example
//! `{"type":"web_search_20250305","name":"web_search"}`. A provider that does not implement the
//! tool ignores or rejects it. The helpers here let the gateway present such a tool to the model as
//! an ordinary custom tool, recognise the resulting `tool_use`, and continue the same turn with the
//! tool result before returning one finished message to the client.
//!
//! Everything in this module is plain data manipulation. Executing the tool and re-sending the
//! request belong to the caller.

use std::collections::{BTreeMap, HashSet};

use bytes::{Bytes, BytesMut};
use serde_json::{Map, Value, json};

use crate::types::messages::{ContentBlock, ContentPart, Request, RequestMessage};

#[cfg(test)]
#[path = "server_tools_tests.rs"]
mod tests;

/// How a declared server tool `type` is matched against an operator mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeMatch {
	Exact(String),
	Prefix(String),
}

impl TypeMatch {
	/// `web_search_*` matches any type starting with `web_search_`; other patterns match exactly.
	pub fn parse(pattern: &str) -> Self {
		match pattern.strip_suffix('*') {
			Some(prefix) => TypeMatch::Prefix(prefix.to_string()),
			None => TypeMatch::Exact(pattern.to_string()),
		}
	}

	pub fn matches(&self, tool_type: &str) -> bool {
		match self {
			TypeMatch::Exact(t) => t == tool_type,
			TypeMatch::Prefix(p) => tool_type.starts_with(p.as_str()),
		}
	}
}

/// A server tool the client declared and the gateway will fulfil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterceptedTool {
	/// The name the client declared. The model calls the tool by this name.
	pub name: String,
	/// The declared server tool `type`.
	pub tool_type: String,
	/// Index of the matching entry in the operator's mapping list.
	pub mapping: usize,
	/// Client-declared `max_uses`, when present.
	pub max_uses: Option<u64>,
}

/// The custom tool definition the model sees in place of the server tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
	pub description: Option<String>,
	pub input_schema: Value,
}

fn server_tool_parts(tool: &Value) -> Option<(&str, &str)> {
	let obj = tool.as_object()?;
	if obj.contains_key("input_schema") {
		return None;
	}
	let tool_type = obj.get("type")?.as_str()?;
	let name = obj.get("name")?.as_str()?;
	Some((tool_type, name))
}

/// Find the client-declared server tools that have an operator mapping.
///
/// A server tool is a `tools[]` entry carrying a `type` and no `input_schema`. Custom tools and
/// server tools without a mapping are left alone.
pub fn find_server_tools(req: &Request, matchers: &[TypeMatch]) -> Vec<InterceptedTool> {
	let Some(tools) = req.rest.get("tools").and_then(Value::as_array) else {
		return Vec::new();
	};
	tools
		.iter()
		.filter_map(|tool| {
			let (tool_type, name) = server_tool_parts(tool)?;
			let mapping = matchers.iter().position(|m| m.matches(tool_type))?;
			Some(InterceptedTool {
				name: name.to_string(),
				tool_type: tool_type.to_string(),
				mapping,
				max_uses: tool.get("max_uses").and_then(Value::as_u64),
			})
		})
		.collect()
}

/// Replace each intercepted server tool with the custom tool definition, keeping the client's name
/// and any `cache_control` marker. `tools` and `definitions` are parallel.
pub fn rewrite_server_tools(
	req: &mut Request,
	tools: &[InterceptedTool],
	definitions: &[ToolDefinition],
) {
	let Some(list) = req.rest.get_mut("tools").and_then(Value::as_array_mut) else {
		return;
	};
	for entry in list.iter_mut() {
		let Some((tool_type, name)) = server_tool_parts(entry) else {
			continue;
		};
		let Some(def) = tools
			.iter()
			.zip(definitions)
			.find(|(t, _)| t.name == name && t.tool_type == tool_type)
			.map(|(_, d)| d)
		else {
			continue;
		};
		let mut replacement = Map::new();
		replacement.insert("name".to_string(), Value::String(name.to_string()));
		if let Some(description) = &def.description {
			replacement.insert(
				"description".to_string(),
				Value::String(description.clone()),
			);
		}
		replacement.insert("input_schema".to_string(), def.input_schema.clone());
		if let Some(cache_control) = entry.get("cache_control") {
			replacement.insert("cache_control".to_string(), cache_control.clone());
		}
		*entry = Value::Object(replacement);
	}
}

/// A `tool_use` block emitted by the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
	pub id: String,
	pub name: String,
	pub input: Value,
}

/// All `tool_use` blocks in a Messages response, in content order.
pub fn tool_uses(message: &Value) -> Vec<ToolUse> {
	message
		.get("content")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|block| {
			let obj = block.as_object()?;
			if obj.get("type")?.as_str()? != "tool_use" {
				return None;
			}
			Some(ToolUse {
				id: obj.get("id")?.as_str()?.to_string(),
				name: obj.get("name")?.as_str()?.to_string(),
				input: obj
					.get("input")
					.cloned()
					.unwrap_or_else(|| Value::Object(Map::new())),
			})
		})
		.collect()
}

pub fn stop_reason(message: &Value) -> Option<&str> {
	message.get("stop_reason")?.as_str()
}

/// Remove `tool_use` blocks for the named tools. When the message stopped for a tool call and no
/// `tool_use` block remains, the stop reason becomes `end_turn`.
pub fn strip_tool_uses(message: &mut Value, names: &HashSet<&str>) -> usize {
	let removed = {
		let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
			return 0;
		};
		let before = content.len();
		content.retain(|block| {
			let is_tool_use = block.get("type").and_then(Value::as_str) == Some("tool_use");
			let ours = block
				.get("name")
				.and_then(Value::as_str)
				.is_some_and(|n| names.contains(n));
			!(is_tool_use && ours)
		});
		before - content.len()
	};
	if removed > 0 && stop_reason(message) == Some("tool_use") && tool_uses(message).is_empty() {
		message["stop_reason"] = Value::String("end_turn".to_string());
	}
	removed
}

/// A stable identity for the set of tool calls in one message, used to stop a model that keeps
/// asking for the exact same call.
pub fn fingerprint(calls: &[ToolUse]) -> String {
	let mut parts: Vec<String> = calls
		.iter()
		.map(|c| format!("{}:{}", c.name, c.input))
		.collect();
	parts.sort();
	parts.join("|")
}

/// Token usage summed across the model calls of one client turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageTotals {
	pub input_tokens: u64,
	pub output_tokens: u64,
	pub cache_creation_input_tokens: Option<u64>,
	pub cache_read_input_tokens: Option<u64>,
}

fn add_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
	match (a, b) {
		(None, None) => None,
		(a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
	}
}

impl UsageTotals {
	pub fn of(message: &Value) -> Self {
		let usage = message.get("usage");
		let field = |name: &str| usage.and_then(|u| u.get(name)).and_then(Value::as_u64);
		Self {
			input_tokens: field("input_tokens").unwrap_or(0),
			output_tokens: field("output_tokens").unwrap_or(0),
			cache_creation_input_tokens: field("cache_creation_input_tokens"),
			cache_read_input_tokens: field("cache_read_input_tokens"),
		}
	}

	pub fn add(&mut self, other: UsageTotals) {
		self.input_tokens += other.input_tokens;
		self.output_tokens += other.output_tokens;
		self.cache_creation_input_tokens = add_opt(
			self.cache_creation_input_tokens,
			other.cache_creation_input_tokens,
		);
		self.cache_read_input_tokens =
			add_opt(self.cache_read_input_tokens, other.cache_read_input_tokens);
	}

	fn apply_input(&self, usage: &mut Value) {
		if !usage.is_object() {
			*usage = Value::Object(Map::new());
		}
		usage["input_tokens"] = Value::from(self.input_tokens);
		if let Some(v) = self.cache_creation_input_tokens {
			usage["cache_creation_input_tokens"] = Value::from(v);
		}
		if let Some(v) = self.cache_read_input_tokens {
			usage["cache_read_input_tokens"] = Value::from(v);
		}
	}

	fn apply(&self, usage: &mut Value) {
		self.apply_input(usage);
		usage["output_tokens"] = Value::from(self.output_tokens);
	}
}

/// Overwrite the usage of a Messages response with the summed totals.
pub fn set_usage(message: &mut Value, totals: UsageTotals) {
	if !message.is_object() {
		return;
	}
	totals.apply(&mut message["usage"]);
}

/// Append the model's turn and the tool results to the conversation, ready to be sent again.
pub fn append_tool_turn(
	req: &mut Request,
	assistant_content: Vec<Value>,
	tool_results: Vec<Value>,
) {
	let parts = |blocks: Vec<Value>| {
		Some(ContentBlock::Array(
			blocks.into_iter().map(ContentPart::Unknown).collect(),
		))
	};
	req.messages.push(RequestMessage {
		role: "assistant".to_string(),
		content: parts(assistant_content),
		rest: Default::default(),
	});
	req.messages.push(RequestMessage {
		role: "user".to_string(),
		content: parts(tool_results),
		rest: Default::default(),
	});
}

/// Build a `tool_result` block from Anthropic content blocks.
pub fn tool_result_block(tool_use_id: &str, content: Vec<Value>, is_error: bool) -> Value {
	let mut block = json!({
		"type": "tool_result",
		"tool_use_id": tool_use_id,
		"content": content,
	});
	if is_error {
		block["is_error"] = Value::Bool(true);
	}
	block
}

pub(crate) const TRUNCATION_MARKER: &str = "\n[tool result truncated by the gateway]";

/// Convert MCP `CallToolResult.content` items into Anthropic `tool_result` content blocks.
///
/// Text and embedded text resources become `text` blocks, images become base64 `image` blocks,
/// and anything else is rendered as its JSON. The total size is capped at `max_bytes`; text is cut
/// at the cap and later items are dropped.
pub fn mcp_content_to_tool_result(items: &[Value], max_bytes: usize) -> Vec<Value> {
	let mut out = Vec::new();
	let mut budget = max_bytes;
	for item in items {
		let (text, image) = match item.get("type").and_then(Value::as_str) {
			Some("text") => (
				item.get("text").and_then(Value::as_str).map(str::to_string),
				None,
			),
			Some("image") => (None, Some(item)),
			Some("resource") => {
				let resource = item.get("resource");
				let text = resource
					.and_then(|r| r.get("text"))
					.and_then(Value::as_str)
					.map(str::to_string)
					.or_else(|| resource.map(|r| r.to_string()));
				(text, None)
			},
			_ => (Some(item.to_string()), None),
		};
		if let Some(image) = image {
			let data = image
				.get("data")
				.and_then(Value::as_str)
				.unwrap_or_default();
			if data.len() > budget {
				out.push(json!({"type": "text", "text": TRUNCATION_MARKER.trim_start()}));
				break;
			}
			budget -= data.len();
			out.push(json!({
				"type": "image",
				"source": {
					"type": "base64",
					"media_type": image.get("mimeType").and_then(Value::as_str).unwrap_or("image/png"),
					"data": data,
				},
			}));
			continue;
		}
		let Some(mut text) = text else {
			continue;
		};
		if text.len() > budget {
			let mut cut = budget;
			while cut > 0 && !text.is_char_boundary(cut) {
				cut -= 1;
			}
			text.truncate(cut);
			text.push_str(TRUNCATION_MARKER);
			out.push(json!({"type": "text", "text": text}));
			break;
		}
		budget -= text.len();
		out.push(json!({"type": "text", "text": text}));
	}
	out
}

/// One server-sent event, as parsed from a buffered stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
	pub event: Option<String>,
	pub data: String,
}

/// Parse buffered SSE bytes into events. Comments and unknown fields are ignored; multi-line data
/// is joined with newlines as the SSE specification requires.
pub fn parse_sse(bytes: &[u8]) -> Vec<SseEvent> {
	let text = String::from_utf8_lossy(bytes);
	let mut events = Vec::new();
	let mut event: Option<String> = None;
	let mut data: Vec<&str> = Vec::new();
	let mut flush = |event: &mut Option<String>, data: &mut Vec<&str>| {
		if !data.is_empty() {
			events.push(SseEvent {
				event: event.take(),
				data: data.join("\n"),
			});
		}
		*event = None;
		data.clear();
	};
	for raw in text.split('\n') {
		let line = raw.strip_suffix('\r').unwrap_or(raw);
		if line.is_empty() {
			flush(&mut event, &mut data);
			continue;
		}
		if line.starts_with(':') {
			continue;
		}
		let (field, value) = match line.split_once(':') {
			Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
			None => (line, ""),
		};
		match field {
			"event" => event = Some(value.to_string()),
			"data" => data.push(value),
			_ => {},
		}
	}
	flush(&mut event, &mut data);
	events
}

/// Encode events back into SSE bytes.
pub fn encode_sse(events: &[SseEvent]) -> Bytes {
	let mut out = BytesMut::new();
	for ev in events {
		out.extend_from_slice(&crate::parse::encode_sse_event(
			ev.event.as_deref().unwrap_or(""),
			Bytes::from(ev.data.clone()),
		));
	}
	out.freeze()
}

fn encode_one(name: &str, data: Value) -> Bytes {
	crate::parse::encode_sse_event(name, Bytes::from(data.to_string()))
}

/// The keepalive event sent while a turn is held back.
pub fn ping_event() -> Bytes {
	encode_one("ping", json!({"type": "ping"}))
}

/// An Anthropic-style `error` event.
pub fn error_event(message: &str) -> Bytes {
	encode_one(
		"error",
		json!({"type": "error", "error": {"type": "api_error", "message": message}}),
	)
}

fn event_type<'a>(ev: &'a SseEvent, data: &'a Value) -> Option<&'a str> {
	ev.event
		.as_deref()
		.or_else(|| data.get("type").and_then(Value::as_str))
}

/// Rebuilds a complete Messages response from its streaming events.
#[derive(Debug, Default)]
pub struct MessageAccumulator {
	message: Option<Value>,
	blocks: BTreeMap<u64, Value>,
	partial_json: BTreeMap<u64, String>,
	complete: bool,
}

impl MessageAccumulator {
	pub fn feed(&mut self, ev: &SseEvent) {
		let Ok(data) = serde_json::from_str::<Value>(&ev.data) else {
			return;
		};
		match event_type(ev, &data) {
			Some("message_start") => {
				let mut message = data.get("message").cloned().unwrap_or(Value::Null);
				if !message.is_object() {
					message = Value::Object(Map::new());
				}
				message["content"] = Value::Array(Vec::new());
				self.message = Some(message);
			},
			Some("content_block_start") => {
				let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
				let mut block = data.get("content_block").cloned().unwrap_or(Value::Null);
				if !block.is_object() {
					return;
				}
				match block.get("type").and_then(Value::as_str) {
					Some("text") => {
						if !block.get("text").is_some_and(Value::is_string) {
							block["text"] = Value::String(String::new());
						}
					},
					Some("thinking") => {
						if !block.get("thinking").is_some_and(Value::is_string) {
							block["thinking"] = Value::String(String::new());
						}
					},
					Some("tool_use") | Some("server_tool_use") => {
						self.partial_json.insert(index, String::new());
					},
					_ => {},
				}
				self.blocks.insert(index, block);
			},
			Some("content_block_delta") => {
				let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
				let Some(delta) = data.get("delta") else {
					return;
				};
				let Some(block) = self.blocks.get_mut(&index) else {
					return;
				};
				match delta.get("type").and_then(Value::as_str) {
					Some("text_delta") => {
						if let (Some(Value::String(text)), Some(add)) = (
							block.get_mut("text"),
							delta.get("text").and_then(Value::as_str),
						) {
							text.push_str(add);
						}
					},
					Some("input_json_delta") => {
						if let Some(add) = delta.get("partial_json").and_then(Value::as_str) {
							self.partial_json.entry(index).or_default().push_str(add);
						}
					},
					Some("thinking_delta") => {
						if let (Some(Value::String(thinking)), Some(add)) = (
							block.get_mut("thinking"),
							delta.get("thinking").and_then(Value::as_str),
						) {
							thinking.push_str(add);
						}
					},
					Some("signature_delta") => {
						if let Some(signature) = delta.get("signature") {
							block["signature"] = signature.clone();
						}
					},
					Some("citations_delta") => {
						if let Some(citation) = delta.get("citation") {
							let citations = block
								.as_object_mut()
								.expect("block is an object")
								.entry("citations")
								.or_insert_with(|| Value::Array(Vec::new()));
							if let Some(list) = citations.as_array_mut() {
								list.push(citation.clone());
							}
						}
					},
					_ => {},
				}
			},
			Some("content_block_stop") => {
				let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
				if let Some(partial) = self.partial_json.remove(&index)
					&& let Some(block) = self.blocks.get_mut(&index)
				{
					let input = if partial.trim().is_empty() {
						block
							.get("input")
							.cloned()
							.unwrap_or_else(|| Value::Object(Map::new()))
					} else {
						serde_json::from_str(&partial).unwrap_or_else(|_| json!({ "_raw": partial }))
					};
					block["input"] = input;
				}
			},
			Some("message_delta") => {
				let Some(message) = self.message.as_mut() else {
					return;
				};
				if let Some(delta) = data.get("delta").and_then(Value::as_object) {
					for (k, v) in delta {
						message[k] = v.clone();
					}
				}
				if let Some(usage) = data.get("usage").and_then(Value::as_object) {
					if !message.get("usage").is_some_and(Value::is_object) {
						message["usage"] = Value::Object(Map::new());
					}
					for (k, v) in usage {
						if !v.is_null() {
							message["usage"][k] = v.clone();
						}
					}
				}
			},
			Some("message_stop") => self.complete = true,
			_ => {},
		}
	}

	pub fn feed_all(&mut self, events: &[SseEvent]) {
		for ev in events {
			self.feed(ev);
		}
	}

	/// True once `message_stop` was seen.
	pub fn is_complete(&self) -> bool {
		self.complete
	}

	/// The rebuilt message, or `None` when no `message_start` was seen.
	pub fn finish(mut self) -> Option<Value> {
		let mut message = self.message.take()?;
		for (index, partial) in std::mem::take(&mut self.partial_json) {
			if let Some(block) = self.blocks.get_mut(&index)
				&& !partial.trim().is_empty()
			{
				block["input"] =
					serde_json::from_str(&partial).unwrap_or_else(|_| json!({ "_raw": partial }));
			}
		}
		message["content"] = Value::Array(self.blocks.into_values().collect());
		Some(message)
	}
}

fn sse(name: &str, data: Value) -> SseEvent {
	SseEvent {
		event: Some(name.to_string()),
		data: data.to_string(),
	}
}

/// Render a complete Messages response as the event sequence a streaming client expects.
pub fn synthesize_sse(message: &Value) -> Vec<SseEvent> {
	let mut events = Vec::new();
	let usage = message.get("usage").cloned().unwrap_or(Value::Null);
	let mut start = message.clone();
	start["content"] = Value::Array(Vec::new());
	start["stop_reason"] = Value::Null;
	start["stop_sequence"] = Value::Null;
	if start.get("usage").is_some_and(Value::is_object) {
		start["usage"]["output_tokens"] = Value::from(0);
	}
	events.push(sse(
		"message_start",
		json!({"type": "message_start", "message": start}),
	));

	let blocks = message
		.get("content")
		.and_then(Value::as_array)
		.cloned()
		.unwrap_or_default();
	for (index, block) in blocks.iter().enumerate() {
		let mut deltas = Vec::new();
		let start_block = match block.get("type").and_then(Value::as_str) {
			Some("text") => {
				deltas.push(json!({
					"type": "text_delta",
					"text": block.get("text").cloned().unwrap_or_else(|| Value::String(String::new())),
				}));
				let mut b = block.clone();
				b["text"] = Value::String(String::new());
				b
			},
			Some("tool_use") => {
				deltas.push(json!({
					"type": "input_json_delta",
					"partial_json": block.get("input").map(Value::to_string).unwrap_or_else(|| "{}".to_string()),
				}));
				let mut b = block.clone();
				b["input"] = Value::Object(Map::new());
				b
			},
			Some("thinking") => {
				if let Some(thinking) = block.get("thinking").and_then(Value::as_str)
					&& !thinking.is_empty()
				{
					deltas.push(json!({"type": "thinking_delta", "thinking": thinking}));
				}
				if let Some(signature) = block.get("signature").and_then(Value::as_str)
					&& !signature.is_empty()
				{
					deltas.push(json!({"type": "signature_delta", "signature": signature}));
				}
				let mut b = block.clone();
				b["thinking"] = Value::String(String::new());
				b["signature"] = Value::String(String::new());
				b
			},
			_ => block.clone(),
		};
		events.push(sse(
			"content_block_start",
			json!({"type": "content_block_start", "index": index, "content_block": start_block}),
		));
		for delta in deltas {
			events.push(sse(
				"content_block_delta",
				json!({"type": "content_block_delta", "index": index, "delta": delta}),
			));
		}
		events.push(sse(
			"content_block_stop",
			json!({"type": "content_block_stop", "index": index}),
		));
	}

	events.push(sse(
		"message_delta",
		json!({
			"type": "message_delta",
			"delta": {
				"stop_reason": message.get("stop_reason").cloned().unwrap_or(Value::Null),
				"stop_sequence": message.get("stop_sequence").cloned().unwrap_or(Value::Null),
			},
			"usage": usage,
		}),
	));
	events.push(sse("message_stop", json!({"type": "message_stop"})));
	events
}

/// Rewrite the usage carried by `message_start` and `message_delta` events to the summed totals.
pub fn patch_usage(events: &mut [SseEvent], totals: UsageTotals) {
	for ev in events.iter_mut() {
		let Ok(mut data) = serde_json::from_str::<Value>(&ev.data) else {
			continue;
		};
		let changed = match event_type(ev, &data) {
			Some("message_start") => {
				if data.get("message").is_some_and(Value::is_object) {
					totals.apply_input(&mut data["message"]["usage"]);
					true
				} else {
					false
				}
			},
			Some("message_delta") => {
				if data.get("usage").is_some_and(Value::is_object) {
					data["usage"]["output_tokens"] = Value::from(totals.output_tokens);
					if data["usage"].get("input_tokens").is_some() {
						totals.apply_input(&mut data["usage"]);
					}
				} else {
					data["usage"] = json!({"output_tokens": totals.output_tokens});
				}
				true
			},
			_ => false,
		};
		if changed {
			ev.data = data.to_string();
		}
	}
}
