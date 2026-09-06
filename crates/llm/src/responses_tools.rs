//! Server-side tool fulfilment for the OpenAI Responses API.
//!
//! Responses clients can declare built-in tools the provider executes, such as
//! `{"type": "web_search"}` or `{"type": "file_search"}`, and remote MCP servers as
//! `{"type": "mcp", "server_label": "docs", "server_url": "https://..."}`. A provider that does
//! not implement them ignores or rejects them. The helpers here let the gateway present such tools
//! to the model as ordinary function tools, recognise the resulting `function_call` items, and
//! continue the same turn with the tool outputs before returning one finished response to the
//! client.
//!
//! Everything in this module is plain data manipulation. Executing the tool and re-sending the
//! request belong to the caller.

use std::collections::{BTreeMap, HashSet};

use bytes::Bytes;
use serde_json::{Map, Value, json};

use crate::server_tools::{
	InterceptedTool, SseEvent, TRUNCATION_MARKER, ToolDefinition, ToolUse, TypeMatch,
};
use crate::types::responses::{RawInputItem, Request, RequestInput};

#[cfg(test)]
#[path = "responses_tools_tests.rs"]
mod tests;

/// Tool entry types the client executes itself. They are never rewritten.
const CLIENT_TOOL_TYPES: &[&str] = &["function", "custom"];

use crate::server_tools::is_client_executed;

fn tools_array(req: &Request) -> Option<&Vec<Value>> {
	req.rest.get("tools").and_then(Value::as_array)
}

fn tool_type(tool: &Value) -> Option<&str> {
	tool.get("type").and_then(Value::as_str)
}

fn string_list(value: Option<&Value>) -> Option<Vec<String>> {
	value.and_then(Value::as_array).map(|items| {
		items
			.iter()
			.filter_map(Value::as_str)
			.map(str::to_string)
			.collect()
	})
}

/// The names of the client's own function tools. The model calls those by name, so the gateway
/// never shadows them.
pub fn client_tool_names(req: &Request) -> HashSet<String> {
	tools_array(req)
		.into_iter()
		.flatten()
		.filter(|tool| tool_type(tool).is_some_and(|ty| CLIENT_TOOL_TYPES.contains(&ty)))
		.filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
		.collect()
}

/// Find the client-declared built-in tools that have an operator mapping.
///
/// A built-in tool is a `tools[]` entry whose `type` is neither a client tool type nor `mcp`. The
/// model calls the replacement by the built-in's type name, so a built-in whose type collides with
/// one of the client's function names is left alone, and so is any type in the `client_executed`
/// list.
pub fn find_builtin_tools(
	req: &Request,
	matchers: &[TypeMatch],
	client_executed: &[TypeMatch],
) -> Vec<InterceptedTool> {
	let taken = client_tool_names(req);
	let mut seen = HashSet::new();
	tools_array(req)
		.into_iter()
		.flatten()
		.filter_map(|tool| {
			let ty = tool_type(tool)?;
			if CLIENT_TOOL_TYPES.contains(&ty) || ty == "mcp" {
				return None;
			}
			let mapping = matchers.iter().position(|m| m.matches(ty))?;
			if is_client_executed(ty, client_executed) {
				tracing::warn!(
					tool_type = ty,
					"server tool mapping matches a client-executed tool; leaving it to the client"
				);
				return None;
			}
			if taken.contains(ty) || !seen.insert(ty.to_string()) {
				return None;
			}
			Some(InterceptedTool {
				name: ty.to_string(),
				tool_type: ty.to_string(),
				mapping,
				max_uses: None,
			})
		})
		.collect()
}

/// The built-in tool types that no mapping covers and that are not in the `client_executed`
/// list.
pub fn unmapped_builtin_tools(
	req: &Request,
	matchers: &[TypeMatch],
	client_executed: &[TypeMatch],
) -> Vec<String> {
	tools_array(req)
		.into_iter()
		.flatten()
		.filter_map(|tool| {
			let ty = tool_type(tool)?;
			if CLIENT_TOOL_TYPES.contains(&ty)
				|| ty == "mcp"
				|| is_client_executed(ty, client_executed)
				|| matchers.iter().any(|m| m.matches(ty))
			{
				return None;
			}
			Some(ty.to_string())
		})
		.collect()
}

/// Remove the named function tools, and a `tool_choice` that forces one of them, so the model has
/// to answer without them.
pub fn remove_function_tools(req: &mut Request, names: &HashSet<&str>) {
	if let Some(list) = req.rest.get_mut("tools").and_then(Value::as_array_mut) {
		list.retain(|tool| {
			!(tool_type(tool) == Some("function")
				&& tool
					.get("name")
					.and_then(Value::as_str)
					.is_some_and(|n| names.contains(n)))
		});
	}
	let forced = req
		.rest
		.get("tool_choice")
		.and_then(|c| c.get("name"))
		.and_then(Value::as_str)
		.is_some_and(|n| names.contains(n));
	if forced && let Some(rest) = req.rest.as_object_mut() {
		rest.remove("tool_choice");
	}
}

/// How a client wants calls to a remote MCP server approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approval {
	Never,
	Always,
	/// Per-tool lists; tools in neither list require approval.
	PerTool {
		never: Vec<String>,
		always: Vec<String>,
	},
}

/// A `{"type": "mcp"}` entry in the client's tool list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpDescriptor {
	/// Position in the client's `tools` array.
	pub index: usize,
	pub server_label: String,
	pub server_url: Option<String>,
	/// The client's `allowed_tools`, when it restricts the server's tools.
	pub allowed_tools: Option<Vec<String>>,
	pub approval: Approval,
}

impl McpDescriptor {
	/// Whether the client requires approval before `tool` runs. The API default is to require it.
	pub fn requires_approval(&self, tool: &str) -> bool {
		match &self.approval {
			Approval::Never => false,
			Approval::Always => true,
			Approval::PerTool { never, .. } => !never.iter().any(|t| t == tool),
		}
	}

	/// Whether the client allows the model to call `tool`.
	pub fn allows(&self, tool: &str) -> bool {
		self
			.allowed_tools
			.as_ref()
			.is_none_or(|list| list.iter().any(|t| t == tool))
	}
}

/// The remote MCP servers the client declared, in tool order.
pub fn find_mcp_descriptors(req: &Request) -> Vec<McpDescriptor> {
	tools_array(req)
		.into_iter()
		.flatten()
		.enumerate()
		.filter_map(|(index, tool)| {
			if tool_type(tool)? != "mcp" {
				return None;
			}
			let allowed_tools = match tool.get("allowed_tools") {
				Some(Value::Object(filter)) => string_list(filter.get("tool_names")),
				other => string_list(other),
			};
			let approval = match tool.get("require_approval") {
				Some(Value::String(mode)) if mode == "never" => Approval::Never,
				Some(Value::Object(modes)) => {
					let names = |key: &str| {
						string_list(modes.get(key).and_then(|v| v.get("tool_names"))).unwrap_or_default()
					};
					Approval::PerTool {
						never: names("never"),
						always: names("always"),
					}
				},
				_ => Approval::Always,
			};
			Some(McpDescriptor {
				index,
				server_label: tool
					.get("server_label")
					.and_then(Value::as_str)
					.unwrap_or_default()
					.to_string(),
				server_url: tool
					.get("server_url")
					.and_then(Value::as_str)
					.map(str::to_string),
				allowed_tools,
				approval,
			})
		})
		.collect()
}

/// The function tool the model sees in place of a built-in or MCP tool.
pub fn function_tool(name: &str, def: &ToolDefinition) -> Value {
	let mut tool = Map::new();
	tool.insert("type".to_string(), json!("function"));
	tool.insert("name".to_string(), json!(name));
	if let Some(description) = &def.description {
		tool.insert("description".to_string(), json!(description));
	}
	tool.insert("parameters".to_string(), def.input_schema.clone());
	Value::Object(tool)
}

/// Replace each intercepted built-in tool with its function tool definition. `tools` and
/// `definitions` are parallel. Entries are replaced in place, so tool indices do not move.
pub fn rewrite_builtin_tools(
	req: &mut Request,
	tools: &[InterceptedTool],
	definitions: &[ToolDefinition],
) {
	let Some(list) = req.rest.get_mut("tools").and_then(Value::as_array_mut) else {
		return;
	};
	for entry in list.iter_mut() {
		let Some(ty) = tool_type(entry) else {
			continue;
		};
		let Some((tool, def)) = tools
			.iter()
			.zip(definitions)
			.find(|(t, _)| t.tool_type == ty)
		else {
			continue;
		};
		*entry = function_tool(&tool.name, def);
	}
}

/// Replace the entries at the given positions of the original `tools` array with the given
/// function tools. Positions refer to the array before any edit.
pub fn replace_tools(req: &mut Request, mut edits: Vec<(usize, Vec<Value>)>) {
	let Some(list) = req.rest.get_mut("tools").and_then(Value::as_array_mut) else {
		return;
	};
	// Later positions first, so earlier ones stay valid while the array changes length.
	edits.sort_by_key(|edit| std::cmp::Reverse(edit.0));
	for (index, replacements) in edits {
		if index < list.len() {
			list.splice(index..=index, replacements);
		}
	}
}

/// All `function_call` items in a response, in output order.
pub fn function_calls(response: &Value) -> Vec<ToolUse> {
	response
		.get("output")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|item| {
			let obj = item.as_object()?;
			if obj.get("type")?.as_str()? != "function_call" {
				return None;
			}
			let arguments = obj.get("arguments").and_then(Value::as_str).unwrap_or("");
			let input = if arguments.trim().is_empty() {
				Value::Object(Map::new())
			} else {
				serde_json::from_str(arguments).unwrap_or_else(|_| json!({ "_raw": arguments }))
			};
			Some(ToolUse {
				id: obj.get("call_id")?.as_str()?.to_string(),
				name: obj.get("name")?.as_str()?.to_string(),
				input,
			})
		})
		.collect()
}

/// Remove `function_call` items for the named tools.
pub fn strip_function_calls(response: &mut Value, names: &HashSet<&str>) -> usize {
	let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) else {
		return 0;
	};
	let before = output.len();
	output.retain(|item| {
		let is_call = tool_type(item) == Some("function_call");
		let ours = item
			.get("name")
			.and_then(Value::as_str)
			.is_some_and(|n| names.contains(n));
		!(is_call && ours)
	});
	before - output.len()
}

/// Append the model's output items and the tool outputs to the conversation, ready to be sent
/// again. A plain-text `input` becomes a user message item first.
pub fn append_tool_turn(req: &mut Request, output_items: Vec<Value>, outputs: Vec<Value>) {
	let mut items = match std::mem::replace(&mut req.input, RequestInput::Items(Vec::new())) {
		RequestInput::Text(text) => vec![RawInputItem::from_value(json!({
			"type": "message",
			"role": "user",
			"content": text,
		}))],
		RequestInput::Items(items) => items,
	};
	items.extend(output_items.into_iter().map(RawInputItem::from_value));
	items.extend(outputs.into_iter().map(RawInputItem::from_value));
	req.input = RequestInput::Items(items);
}

/// Build a `function_call_output` item.
pub fn function_call_output(call_id: &str, output: String) -> Value {
	json!({
		"type": "function_call_output",
		"call_id": call_id,
		"output": output,
	})
}

/// Render MCP `CallToolResult.content` items as the text a `function_call_output` carries.
///
/// Text and embedded text resources are joined with newlines, images become a placeholder, and
/// anything else is rendered as its JSON. The result is cut at `max_bytes`.
pub fn mcp_content_to_output(items: &[Value], max_bytes: usize) -> String {
	let mut out = String::new();
	for item in items {
		let text = match tool_type(item) {
			Some("text") => item.get("text").and_then(Value::as_str).map(str::to_string),
			Some("image") => Some(format!(
				"[image {}]",
				item
					.get("mimeType")
					.and_then(Value::as_str)
					.unwrap_or("image/png")
			)),
			Some("resource") => {
				let resource = item.get("resource");
				resource
					.and_then(|r| r.get("text"))
					.and_then(Value::as_str)
					.map(str::to_string)
					.or_else(|| resource.map(Value::to_string))
			},
			_ => Some(item.to_string()),
		};
		let Some(text) = text else {
			continue;
		};
		if !out.is_empty() {
			out.push('\n');
		}
		if out.len() + text.len() > max_bytes {
			let mut cut = max_bytes.saturating_sub(out.len()).min(text.len());
			while cut > 0 && !text.is_char_boundary(cut) {
				cut -= 1;
			}
			out.push_str(&text[..cut]);
			out.push_str(TRUNCATION_MARKER);
			return out;
		}
		out.push_str(&text);
	}
	out
}

/// Token usage summed across the model calls of one client turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageTotals {
	pub input_tokens: u64,
	pub output_tokens: u64,
	pub cached_tokens: Option<u64>,
	pub reasoning_tokens: Option<u64>,
}

fn add_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
	match (a, b) {
		(None, None) => None,
		(a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
	}
}

impl UsageTotals {
	pub fn of(response: &Value) -> Self {
		let usage = response.get("usage");
		let field = |path: &[&str]| {
			let mut value = usage?;
			for key in path {
				value = value.get(key)?;
			}
			value.as_u64()
		};
		Self {
			input_tokens: field(&["input_tokens"]).unwrap_or(0),
			output_tokens: field(&["output_tokens"]).unwrap_or(0),
			cached_tokens: field(&["input_tokens_details", "cached_tokens"]),
			reasoning_tokens: field(&["output_tokens_details", "reasoning_tokens"]),
		}
	}

	pub fn add(&mut self, other: UsageTotals) {
		self.input_tokens += other.input_tokens;
		self.output_tokens += other.output_tokens;
		self.cached_tokens = add_opt(self.cached_tokens, other.cached_tokens);
		self.reasoning_tokens = add_opt(self.reasoning_tokens, other.reasoning_tokens);
	}

	fn apply(&self, usage: &mut Value) {
		if !usage.is_object() {
			*usage = Value::Object(Map::new());
		}
		usage["input_tokens"] = Value::from(self.input_tokens);
		usage["output_tokens"] = Value::from(self.output_tokens);
		usage["total_tokens"] = Value::from(self.input_tokens + self.output_tokens);
		if let Some(cached) = self.cached_tokens {
			if !usage["input_tokens_details"].is_object() {
				usage["input_tokens_details"] = Value::Object(Map::new());
			}
			usage["input_tokens_details"]["cached_tokens"] = Value::from(cached);
		}
		if let Some(reasoning) = self.reasoning_tokens {
			if !usage["output_tokens_details"].is_object() {
				usage["output_tokens_details"] = Value::Object(Map::new());
			}
			usage["output_tokens_details"]["reasoning_tokens"] = Value::from(reasoning);
		}
	}
}

/// Overwrite the usage of a response with the summed totals.
pub fn set_usage(response: &mut Value, totals: UsageTotals) {
	if !response.is_object() {
		return;
	}
	totals.apply(&mut response["usage"]);
}

/// The event type, taken from the JSON body first: some producers put a generic name on the
/// SSE `event` line.
fn event_type<'a>(ev: &'a SseEvent, data: &'a Value) -> Option<&'a str> {
	data
		.get("type")
		.and_then(Value::as_str)
		.or(ev.event.as_deref())
}

fn is_terminal(event_type: &str) -> bool {
	matches!(
		event_type,
		"response.completed" | "response.incomplete" | "response.failed"
	)
}

fn index_of(data: &Value, key: &str) -> u64 {
	data.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn push_str(target: &mut Value, add: &str) {
	match target {
		Value::String(s) => s.push_str(add),
		other => *other = Value::String(add.to_string()),
	}
}

/// Rebuilds a complete Responses response from its streaming events.
///
/// Output items are assembled from `output_item.added`, the content and argument deltas, and
/// `output_item.done`; the terminal event supplies status and usage. That covers producers whose
/// terminal event carries an empty `output` and whose `done` items carry no text.
#[derive(Debug, Default)]
pub struct ResponseAccumulator {
	response: Option<Value>,
	items: BTreeMap<u64, Value>,
	/// Content parts per output item, by content index.
	parts: BTreeMap<u64, BTreeMap<u64, Value>>,
	terminal: Option<Value>,
}

impl ResponseAccumulator {
	pub fn feed(&mut self, ev: &SseEvent) {
		let Ok(data) = serde_json::from_str::<Value>(&ev.data) else {
			return;
		};
		let Some(ty) = event_type(ev, &data) else {
			return;
		};
		match ty {
			"response.created" | "response.in_progress" => {
				if self.response.is_none() {
					self.response = data.get("response").cloned();
				}
			},
			"response.output_item.added" => {
				if let Some(item) = data.get("item") {
					self
						.items
						.entry(index_of(&data, "output_index"))
						.or_insert_with(|| item.clone());
				}
			},
			"response.content_part.added" => {
				if let Some(part) = data.get("part") {
					self
						.parts
						.entry(index_of(&data, "output_index"))
						.or_default()
						.entry(index_of(&data, "content_index"))
						.or_insert_with(|| part.clone());
				}
			},
			"response.output_text.delta" | "response.refusal.delta" => {
				let key = if ty == "response.output_text.delta" {
					"text"
				} else {
					"refusal"
				};
				let part = self
					.parts
					.entry(index_of(&data, "output_index"))
					.or_default()
					.entry(index_of(&data, "content_index"))
					.or_insert_with(|| {
						json!({"type": if key == "text" { "output_text" } else { "refusal" }, key: "", "annotations": []})
					});
				if let Some(delta) = data.get("delta").and_then(Value::as_str) {
					push_str(&mut part[key], delta);
				}
			},
			"response.output_text.done" | "response.refusal.done" => {
				let key = if ty == "response.output_text.done" {
					"text"
				} else {
					"refusal"
				};
				if let Some(text) = data.get(key).and_then(Value::as_str)
					&& !text.is_empty()
					&& let Some(part) = self
						.parts
						.get_mut(&index_of(&data, "output_index"))
						.and_then(|parts| parts.get_mut(&index_of(&data, "content_index")))
				{
					part[key] = Value::String(text.to_string());
				}
			},
			"response.function_call_arguments.delta" => {
				if let (Some(item), Some(delta)) = (
					self.items.get_mut(&index_of(&data, "output_index")),
					data.get("delta").and_then(Value::as_str),
				) {
					push_str(&mut item["arguments"], delta);
				}
			},
			"response.output_item.done" => {
				let index = index_of(&data, "output_index");
				let Some(done) = data.get("item").cloned() else {
					return;
				};
				let merged = match self.items.remove(&index) {
					Some(mut partial) => {
						let keep_arguments = done
							.get("arguments")
							.and_then(Value::as_str)
							.is_none_or(str::is_empty)
							&& partial
								.get("arguments")
								.is_some_and(|a| a.as_str().is_some_and(|s| !s.is_empty()));
						let arguments = partial.get("arguments").cloned();
						partial = done;
						if keep_arguments && let Some(arguments) = arguments {
							partial["arguments"] = arguments;
						}
						partial
					},
					None => done,
				};
				self.items.insert(index, merged);
			},
			_ if is_terminal(ty) => {
				self.terminal = data.get("response").cloned();
			},
			_ => {},
		}
	}

	pub fn feed_all(&mut self, events: &[SseEvent]) {
		for ev in events {
			self.feed(ev);
		}
	}

	/// True once a terminal event was seen.
	pub fn is_complete(&self) -> bool {
		self.terminal.is_some()
	}

	/// The rebuilt response, or `None` when the stream did not finish.
	pub fn finish(mut self) -> Option<Value> {
		let mut response = self.terminal.take()?;
		if !response.is_object() {
			return None;
		}
		let mut items: BTreeMap<u64, Value> = std::mem::take(&mut self.items);
		for (index, parts) in std::mem::take(&mut self.parts) {
			let item = items.entry(index).or_insert_with(
				|| json!({"type": "message", "role": "assistant", "status": "completed", "content": []}),
			);
			let has_content = item
				.get("content")
				.and_then(Value::as_array)
				.is_some_and(|c| !c.is_empty());
			if !has_content {
				item["content"] = Value::Array(parts.into_values().collect());
			}
		}
		let terminal_has_output = response
			.get("output")
			.and_then(Value::as_array)
			.is_some_and(|o| !o.is_empty());
		if !terminal_has_output && !items.is_empty() {
			response["output"] = Value::Array(items.into_values().collect());
		}
		if let Some(base) = self.response.as_ref().and_then(Value::as_object) {
			// Fields the terminal event left out, such as the model, come from the first event.
			if let Some(obj) = response.as_object_mut() {
				for (k, v) in base {
					if !obj.contains_key(k) {
						obj.insert(k.clone(), v.clone());
					}
				}
			}
		}
		Some(response)
	}
}

/// The complete response of a stream, or `None` when the stream did not finish.
pub fn final_response(events: &[SseEvent]) -> Option<Value> {
	let mut acc = ResponseAccumulator::default();
	acc.feed_all(events);
	acc.finish()
}

struct Emitter {
	events: Vec<SseEvent>,
	sequence: u64,
}

impl Emitter {
	fn push(&mut self, name: &str, mut data: Value) {
		data["type"] = json!(name);
		data["sequence_number"] = json!(self.sequence);
		self.sequence += 1;
		self.events.push(SseEvent {
			event: Some(name.to_string()),
			data: data.to_string(),
		});
	}
}

/// Render a complete response as the event sequence a streaming client expects.
pub fn synthesize_sse(response: &Value) -> Vec<SseEvent> {
	let mut out = Emitter {
		events: Vec::new(),
		sequence: 0,
	};
	let mut created = response.clone();
	created["output"] = json!([]);
	created["status"] = json!("in_progress");
	created["usage"] = Value::Null;
	out.push("response.created", json!({ "response": created }));
	out.push("response.in_progress", json!({ "response": created }));

	let items = response
		.get("output")
		.and_then(Value::as_array)
		.cloned()
		.unwrap_or_default();
	for (output_index, item) in items.iter().enumerate() {
		let item_id = item.get("id").cloned().unwrap_or(Value::Null);
		match tool_type(item) {
			Some("message") => {
				let mut shell = item.clone();
				shell["content"] = json!([]);
				shell["status"] = json!("in_progress");
				out.push(
					"response.output_item.added",
					json!({"output_index": output_index, "item": shell}),
				);
				let parts = item
					.get("content")
					.and_then(Value::as_array)
					.cloned()
					.unwrap_or_default();
				for (content_index, part) in parts.iter().enumerate() {
					let at = json!({
						"item_id": item_id,
						"output_index": output_index,
						"content_index": content_index,
					});
					let mut with = |name: &str, extra: Value| {
						let mut data = at.clone();
						if let (Some(obj), Some(more)) = (data.as_object_mut(), extra.as_object()) {
							for (k, v) in more {
								obj.insert(k.clone(), v.clone());
							}
						}
						out.push(name, data);
					};
					match tool_type(part) {
						Some("output_text") => {
							let text = part.get("text").and_then(Value::as_str).unwrap_or("");
							let mut empty = part.clone();
							empty["text"] = json!("");
							with("response.content_part.added", json!({"part": empty}));
							if !text.is_empty() {
								with(
									"response.output_text.delta",
									json!({"delta": text, "logprobs": []}),
								);
							}
							with(
								"response.output_text.done",
								json!({"text": text, "logprobs": []}),
							);
							with("response.content_part.done", json!({"part": part}));
						},
						Some("refusal") => {
							let refusal = part.get("refusal").and_then(Value::as_str).unwrap_or("");
							let mut empty = part.clone();
							empty["refusal"] = json!("");
							with("response.content_part.added", json!({"part": empty}));
							if !refusal.is_empty() {
								with("response.refusal.delta", json!({"delta": refusal}));
							}
							with("response.refusal.done", json!({"refusal": refusal}));
							with("response.content_part.done", json!({"part": part}));
						},
						_ => {
							with("response.content_part.added", json!({"part": part}));
							with("response.content_part.done", json!({"part": part}));
						},
					}
				}
				out.push(
					"response.output_item.done",
					json!({"output_index": output_index, "item": item}),
				);
			},
			Some("function_call") => {
				let arguments = item.get("arguments").and_then(Value::as_str).unwrap_or("");
				let mut shell = item.clone();
				shell["arguments"] = json!("");
				out.push(
					"response.output_item.added",
					json!({"output_index": output_index, "item": shell}),
				);
				out.push(
					"response.function_call_arguments.delta",
					json!({"item_id": item_id, "output_index": output_index, "delta": arguments}),
				);
				out.push(
					"response.function_call_arguments.done",
					json!({"item_id": item_id, "output_index": output_index, "arguments": arguments}),
				);
				out.push(
					"response.output_item.done",
					json!({"output_index": output_index, "item": item}),
				);
			},
			_ => {
				out.push(
					"response.output_item.added",
					json!({"output_index": output_index, "item": item}),
				);
				out.push(
					"response.output_item.done",
					json!({"output_index": output_index, "item": item}),
				);
			},
		}
	}

	let terminal = match response.get("status").and_then(Value::as_str) {
		Some("incomplete") => "response.incomplete",
		Some("failed") => "response.failed",
		_ => "response.completed",
	};
	out.push(terminal, json!({ "response": response }));
	out.events
}

/// Rewrite the usage carried by the stream's terminal event to the summed totals.
pub fn patch_usage(events: &mut [SseEvent], totals: UsageTotals) {
	for ev in events.iter_mut() {
		let Ok(mut data) = serde_json::from_str::<Value>(&ev.data) else {
			continue;
		};
		let Some(ty) = event_type(ev, &data) else {
			continue;
		};
		if !is_terminal(ty) || !data.get("response").is_some_and(Value::is_object) {
			continue;
		}
		totals.apply(&mut data["response"]["usage"]);
		ev.data = data.to_string();
	}
}

/// The comment line sent while a turn is held back. SSE clients ignore comments.
pub fn keepalive() -> Bytes {
	Bytes::from_static(b": keepalive\n\n")
}

/// An `error` event in the Responses stream shape.
pub fn error_event(message: &str) -> Bytes {
	crate::parse::encode_sse_event(
		"error",
		Bytes::from(
			json!({
				"type": "error",
				"code": "server_tool_error",
				"message": message,
				"param": null,
				"sequence_number": 0,
			})
			.to_string(),
		),
	)
}
