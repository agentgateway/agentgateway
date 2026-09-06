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

use std::collections::{BTreeMap, HashMap, HashSet};

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

/// Vendor-defined tool types that share the server tool shape but are executed by the client, on
/// every route: Anthropic's `bash`, `text_editor`, `computer` and `memory` tools, and the OpenAI
/// Responses shell, patch and computer-use tools. This is the default for the operator's
/// `clientExecuted` list; a mapping that matches one of these is ignored unless the operator
/// removes the entry.
pub const DEFAULT_CLIENT_EXECUTED_TOOL_TYPES: &[&str] = &[
	"bash_*",
	"text_editor_*",
	"computer_*",
	"memory_*",
	"local_shell",
	"shell",
	"apply_patch",
	"computer_use_preview",
	"computer",
];

/// Whether a declared tool type is in the client-executed list.
pub fn is_client_executed(tool_type: &str, client_executed: &[TypeMatch]) -> bool {
	client_executed.iter().any(|m| m.matches(tool_type))
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
/// A server tool is a `tools[]` entry carrying a `type` and no `input_schema`. Custom tools,
/// types in the `client_executed` list and server tools without a mapping are left alone.
pub fn find_server_tools(
	req: &Request,
	matchers: &[TypeMatch],
	client_executed: &[TypeMatch],
) -> Vec<InterceptedTool> {
	let Some(tools) = req.rest.get("tools").and_then(Value::as_array) else {
		return Vec::new();
	};
	tools
		.iter()
		.filter_map(|tool| {
			let (tool_type, name) = server_tool_parts(tool)?;
			let mapping = matchers.iter().position(|m| m.matches(tool_type))?;
			if is_client_executed(tool_type, client_executed) {
				tracing::warn!(
					tool_type,
					"server tool mapping matches a client-executed tool; leaving it to the client"
				);
				return None;
			}
			Some(InterceptedTool {
				name: name.to_string(),
				tool_type: tool_type.to_string(),
				mapping,
				max_uses: tool.get("max_uses").and_then(Value::as_u64),
			})
		})
		.collect()
}

/// The types of declared server tools that no mapping covers and that are not in the
/// `client_executed` list, so a provider without them will drop or reject them.
pub fn unmapped_server_tools(
	req: &Request,
	matchers: &[TypeMatch],
	client_executed: &[TypeMatch],
) -> Vec<String> {
	req
		.rest
		.get("tools")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|tool| {
			let (tool_type, _) = server_tool_parts(tool)?;
			if is_client_executed(tool_type, client_executed)
				|| matchers.iter().any(|m| m.matches(tool_type))
			{
				return None;
			}
			Some(tool_type.to_string())
		})
		.collect()
}

/// Remove the named tools from the request, and a `tool_choice` that forces one of them, so the
/// model has to answer without them.
pub fn remove_tools(req: &mut Request, names: &HashSet<&str>) {
	if let Some(list) = req.rest.get_mut("tools").and_then(Value::as_array_mut) {
		list.retain(|tool| {
			!tool
				.get("name")
				.and_then(Value::as_str)
				.is_some_and(|n| names.contains(n))
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

/// One result extracted from a search tool's output.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchResult {
	pub url: String,
	pub title: String,
	pub snippet: String,
}

fn string_field<'a>(obj: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
	keys
		.iter()
		.find_map(|k| obj.get(*k).and_then(Value::as_str))
}

fn result_from_object(obj: &Map<String, Value>) -> Option<SearchResult> {
	let url = string_field(obj, &["url", "link", "href"])?;
	if !url.starts_with("http://") && !url.starts_with("https://") {
		return None;
	}
	Some(SearchResult {
		url: url.to_string(),
		title: string_field(obj, &["title", "name"])
			.unwrap_or_default()
			.to_string(),
		snippet: string_field(
			obj,
			&["snippet", "content", "description", "text", "summary"],
		)
		.unwrap_or_default()
		.to_string(),
	})
}

fn results_from_json(value: &Value) -> Vec<SearchResult> {
	const LIST_KEYS: &[&str] = &[
		"results",
		"items",
		"data",
		"organic",
		"web",
		"hits",
		"documents",
	];
	let from_list = |items: &Vec<Value>| -> Vec<SearchResult> {
		items
			.iter()
			.filter_map(Value::as_object)
			.filter_map(result_from_object)
			.collect()
	};
	match value {
		Value::Array(items) => from_list(items),
		Value::Object(obj) => {
			if let Some(single) = result_from_object(obj) {
				return vec![single];
			}
			for key in LIST_KEYS {
				match obj.get(*key) {
					Some(Value::Array(items)) => {
						let found = from_list(items);
						if !found.is_empty() {
							return found;
						}
					},
					Some(Value::Object(inner)) => {
						let found = results_from_json(&Value::Object(inner.clone()));
						if !found.is_empty() {
							return found;
						}
					},
					_ => {},
				}
			}
			Vec::new()
		},
		_ => Vec::new(),
	}
}

fn labelled_line<'a>(line: &'a str, labels: &[&str]) -> Option<&'a str> {
	let (label, value) = line.split_once(':')?;
	let label = label.trim().to_ascii_lowercase();
	labels.contains(&label.as_str()).then(|| value.trim())
}

fn results_from_text(text: &str) -> Vec<SearchResult> {
	let mut out = Vec::new();
	let mut current = SearchResult::default();
	let flush = |current: &mut SearchResult, out: &mut Vec<SearchResult>| {
		if !current.url.is_empty() {
			out.push(std::mem::take(current));
		} else {
			*current = SearchResult::default();
		}
	};
	for line in text.lines() {
		if line.trim().is_empty() {
			flush(&mut current, &mut out);
			continue;
		}
		if let Some(title) = labelled_line(line, &["title", "name"]) {
			if !current.url.is_empty() {
				flush(&mut current, &mut out);
			}
			current.title = title.to_string();
		} else if let Some(url) = labelled_line(line, &["url", "link", "source"]) {
			if !current.url.is_empty() {
				flush(&mut current, &mut out);
			}
			current.url = url.to_string();
		} else if let Some(snippet) =
			labelled_line(line, &["snippet", "content", "description", "summary"])
		{
			current.snippet = snippet.to_string();
		} else if !current.url.is_empty() && current.snippet.is_empty() {
			current.snippet = line.trim().to_string();
		}
	}
	flush(&mut current, &mut out);
	if !out.is_empty() {
		return out;
	}
	// Last resort: bare links.
	text
		.split_whitespace()
		.filter(|word| word.starts_with("http://") || word.starts_with("https://"))
		.map(|word| SearchResult {
			url: word
				.trim_end_matches(['.', ',', ')', ']', ';', '\'', '"'])
				.to_string(),
			..Default::default()
		})
		.collect()
}

/// Pull search results out of a search tool's text output.
///
/// JSON is tried first: an array, or an object with a `results`, `items`, `data`, `organic`,
/// `web`, `hits` or `documents` list, of objects carrying a `url` or `link`, with `title` or
/// `name`, and `snippet`, `content`, `description`, `text` or `summary`. Otherwise `Title:`,
/// `URL:` and `Snippet:` lines are read, and as a last resort bare links are collected. Returns
/// nothing when the output does not look like search results.
pub fn extract_search_results(text: &str) -> Vec<SearchResult> {
	let trimmed = text.trim();
	if (trimmed.starts_with('{') || trimmed.starts_with('['))
		&& let Ok(value) = serde_json::from_str::<Value>(trimmed)
	{
		let found = results_from_json(&value);
		if !found.is_empty() {
			return found;
		}
	}
	results_from_text(text)
}

/// The `server_tool_use` and `web_search_tool_result` pair a native client expects for one
/// executed search. The result items carry no `encrypted_content`, which only the vendor can
/// mint, and the snippet rides in an additional `snippet` field so the evidence survives a replay.
pub fn web_search_blocks(call: &ToolUse, results: &[SearchResult]) -> (Value, Value) {
	let use_block = json!({
		"type": "server_tool_use",
		"id": call.id,
		"name": call.name,
		"input": call.input,
	});
	let items: Vec<Value> = results
		.iter()
		.map(|r| {
			let mut item = json!({
				"type": "web_search_result",
				"url": r.url,
				"title": r.title,
				"page_age": Value::Null,
				"encrypted_content": "",
			});
			if !r.snippet.is_empty() {
				item["snippet"] = Value::String(r.snippet.clone());
			}
			item
		})
		.collect();
	let result_block = json!({
		"type": "web_search_tool_result",
		"tool_use_id": call.id,
		"content": items,
	});
	(use_block, result_block)
}

/// Insert blocks at the start of a message's content, ahead of the final text.
pub fn prepend_blocks(message: &mut Value, blocks: Vec<Value>) {
	if blocks.is_empty() {
		return;
	}
	let content = match message.get_mut("content") {
		Some(Value::Array(content)) => content,
		_ => {
			message["content"] = Value::Array(Vec::new());
			message["content"].as_array_mut().expect("just set")
		},
	};
	content.splice(0..0, blocks);
}

fn render_search_results(items: &[Value]) -> String {
	items
		.iter()
		.filter_map(Value::as_object)
		.map(|item| {
			let field = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or("");
			let mut line = format!("Title: {}\nURL: {}", field("title"), field("url"));
			let snippet = field("snippet");
			if !snippet.is_empty() {
				line.push_str("\nSnippet: ");
				line.push_str(snippet);
			}
			line
		})
		.collect::<Vec<_>>()
		.join("\n\n")
}

/// Rewrite replayed `server_tool_use` and `web_search_tool_result` pairs whose results carry no
/// `encrypted_content`, the shape the gateway synthesises, into one assistant text block that
/// carries the query and the results. Both blocks sit in the assistant turn, where a provider
/// that never ran the search has no tool call to pair them with, so text is the form every
/// backend reads. Blocks with vendor-encrypted content are left alone.
pub fn flatten_replayed_search_results(req: &mut Request) -> usize {
	let mut rewritten = 0;
	for message in &mut req.messages {
		let Some(ContentBlock::Array(parts)) = message.content.as_mut() else {
			continue;
		};
		// Queries by tool use id, so the text can say what was searched.
		let queries: HashMap<String, String> = parts
			.iter()
			.filter_map(|part| match part {
				ContentPart::Unknown(block)
					if block.get("type").and_then(Value::as_str) == Some("server_tool_use") =>
				{
					Some((
						block.get("id")?.as_str()?.to_string(),
						block
							.get("input")
							.and_then(|i| i.get("query"))
							.and_then(Value::as_str)
							.unwrap_or_default()
							.to_string(),
					))
				},
				_ => None,
			})
			.collect();
		let mut flattened_ids: HashSet<String> = HashSet::new();
		for part in parts.iter_mut() {
			let ContentPart::Unknown(block) = part else {
				continue;
			};
			if block.get("type").and_then(Value::as_str) != Some("web_search_tool_result") {
				continue;
			}
			let Some(tool_use_id) = block
				.get("tool_use_id")
				.and_then(Value::as_str)
				.map(str::to_string)
			else {
				continue;
			};
			let body = match block.get("content") {
				Some(Value::Array(items)) => {
					let encrypted = items.iter().any(|item| {
						item
							.get("encrypted_content")
							.and_then(Value::as_str)
							.is_some_and(|c| !c.is_empty())
					});
					if encrypted {
						continue;
					}
					if items.is_empty() {
						"No results.".to_string()
					} else {
						render_search_results(items)
					}
				},
				Some(Value::Object(error)) => format!(
					"Search failed: {}",
					error
						.get("error_code")
						.and_then(Value::as_str)
						.unwrap_or("unknown error")
				),
				_ => continue,
			};
			let query = queries.get(&tool_use_id).cloned().unwrap_or_default();
			let heading = if query.is_empty() {
				"Web search results:".to_string()
			} else {
				format!("Web search for \"{query}\":")
			};
			*part = ContentPart::Text {
				r#type: "text".to_string(),
				text: format!("{heading}\n{body}"),
				rest: Default::default(),
			};
			flattened_ids.insert(tool_use_id);
			rewritten += 1;
		}
		if !flattened_ids.is_empty() {
			parts.retain(|part| match part {
				ContentPart::Unknown(block) => {
					!(block.get("type").and_then(Value::as_str) == Some("server_tool_use")
						&& block
							.get("id")
							.and_then(Value::as_str)
							.is_some_and(|id| flattened_ids.contains(id)))
				},
				_ => true,
			});
		}
	}
	rewritten
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
			Some("tool_use" | "server_tool_use") => {
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
