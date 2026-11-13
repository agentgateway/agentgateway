//! Agent traffic shared by the IR suite and the legacy comparison in `agent-llm`, and the checks
//! that a converted body still carries its content.
//!
//! Requests are agent conversations: a system prompt, four tools, a question, then tool turns
//! until the body reaches its size. Each turn is an assistant step that calls `read_file` and the
//! file it returns, about 5 KiB of code with quotes, escapes, tabs and some non-ASCII. Replies
//! are prose followed by one `read_file` call; streams send about one token per event, each in
//! its own transport read, as a provider flushes them.
//!
//! Not covered: reasoning and the replay state it carries between turns, images, parallel tool
//! calls, and provider-executed tools.

#![allow(dead_code)]

use std::fmt;

use agent_llm_ir::{
	ClientEndpoint, ClientProtocol, Delivery, IdentifierPseudonymizer, ReplyContext, StateScope,
	Upstream, UpstreamProtocol,
};
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
	Messages,
	Responses,
	Chat,
	Gemini,
}

pub const PROTOCOLS: &[Protocol] = &[
	Protocol::Messages,
	Protocol::Responses,
	Protocol::Chat,
	Protocol::Gemini,
];

/// What reply bodies may hold, as the gateway allows by default for a buffered body.
pub const LIMIT: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Size(pub usize);

impl fmt::Display for Size {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.0 {
			n if n >= 1 << 20 => write!(f, "{}MiB", n >> 20),
			n => write!(f, "{}KiB", n >> 10),
		}
	}
}

/// A client protocol served by a provider protocol. The same protocol at both ends is a
/// passthrough.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pair {
	pub client: Protocol,
	pub provider: Protocol,
}

impl Pair {
	pub const fn new(client: Protocol, provider: Protocol) -> Self {
		Self { client, provider }
	}

	pub fn is_passthrough(self) -> bool {
		self.client == self.provider
	}

	/// The model sent to the provider.
	pub const fn model(self) -> &'static str {
		self.provider.model()
	}

	/// As the gateway builds it for this provider.
	pub fn upstream(self) -> Upstream<'static> {
		let protocol = self.provider.upstream();
		Upstream {
			identifier_pseudonymizer: &FixedIdentifier,
			scope: scope(self.model()),
			protocol,
			// As the gateway sets for a Messages provider when the client sends no limit.
			default_max_tokens: matches!(protocol, UpstreamProtocol::Messages)
				.then(|| std::num::NonZeroU64::new(32_000).unwrap()),
		}
	}

	/// As the gateway builds it for a reply to a request with function tools only.
	pub const fn reply_context(self) -> ReplyContext<'static> {
		ReplyContext {
			client: self.client.client(),
			upstream: self.provider.upstream(),
			scope: scope(self.model()),
			routes: &[],
			show_reasoning: false,
			include_usage: false,
			created: 1_700_000_000,
			fallback_reply_id: "reply-1",
			stop_sequences: Vec::new(),
			max_held_bytes: LIMIT,
		}
	}
}

impl fmt::Display for Pair {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}>{}", self.client.name(), self.provider.name())
	}
}

const fn scope(model: &'static str) -> StateScope<'static> {
	StateScope {
		backend: "bench",
		provider: "bench",
		model,
	}
}

impl Protocol {
	pub const fn name(self) -> &'static str {
		match self {
			Self::Messages => "messages",
			Self::Responses => "responses",
			Self::Chat => "chat",
			Self::Gemini => "gemini",
		}
	}

	pub const fn model(self) -> &'static str {
		match self {
			Self::Messages => "claude-sonnet-4-5",
			Self::Responses => "gpt-5",
			Self::Chat => "gpt-4.1",
			Self::Gemini => "gemini-2.5-pro",
		}
	}

	pub const fn client(self) -> ClientProtocol {
		match self {
			Self::Messages => ClientProtocol::Messages,
			Self::Responses => ClientProtocol::Responses,
			Self::Chat => ClientProtocol::ChatCompletions,
			Self::Gemini => ClientProtocol::Gemini,
		}
	}

	pub const fn endpoint(self, delivery: Delivery) -> ClientEndpoint {
		match self {
			Self::Messages => ClientEndpoint::Messages,
			Self::Responses => ClientEndpoint::Responses,
			Self::Chat => ClientEndpoint::ChatCompletions,
			Self::Gemini => ClientEndpoint::Gemini(delivery),
		}
	}

	pub const fn upstream(self) -> UpstreamProtocol {
		match self {
			Self::Messages => UpstreamProtocol::Messages,
			// As the gateway sets for OpenAI.
			Self::Responses => UpstreamProtocol::Responses {
				safety_identifier: true,
				reasoning_context: false,
			},
			Self::Chat => UpstreamProtocol::ChatCompletions,
			Self::Gemini => UpstreamProtocol::Gemini,
		}
	}
}

/// Placeholder pseudonymizer for benchmark inputs without long identifiers.
struct FixedIdentifier;

impl IdentifierPseudonymizer for FixedIdentifier {
	fn pseudonymize(&self, _: &str) -> String {
		"01".into()
	}
}

// --- Checks ---

/// A converted request must carry every tool turn: its step, its call and its result.
pub fn check_request(name: &str, turns: usize, out: &[u8]) {
	check_request_lines(name, turns, FILE_LINES_PER_TURN, out);
}

pub fn check_request_lines(name: &str, turns: usize, lines: usize, out: &[u8]) {
	let _: Value = serde_json::from_slice(out).expect("valid JSON");
	let out = std::str::from_utf8(out).unwrap();
	assert_eq!(
		out.matches("STEP-").count(),
		turns,
		"{name}: assistant turns"
	);
	assert_eq!(out.matches("FILE-").count(), turns, "{name}: tool results");
	assert_eq!(
		out.matches("module_").count(),
		turns * (1 + lines),
		"{name}: tool calls and their results"
	);
}

/// A client's whole reply must carry `text` and the `read_file` call.
pub fn check_buffered(name: &str, client: Protocol, text: &str, out: &[u8]) {
	let out: Value = serde_json::from_slice(out)
		.unwrap_or_else(|err| panic!("{name}: {err} in {}", String::from_utf8_lossy(out)));
	let (got, tool) = match client {
		Protocol::Messages => (
			out["content"][0]["text"].as_str().unwrap().to_string(),
			out["content"][1]["name"].clone(),
		),
		Protocol::Chat => (
			out["choices"][0]["message"]["content"]
				.as_str()
				.unwrap()
				.to_string(),
			out["choices"][0]["message"]["tool_calls"][0]["function"]["name"].clone(),
		),
		Protocol::Responses => {
			let output = out["output"].as_array().unwrap();
			let message = output
				.iter()
				.find(|item| item["type"] == "message")
				.unwrap();
			let call = output
				.iter()
				.find(|item| item["type"] == "function_call")
				.unwrap();
			(
				message["content"][0]["text"].as_str().unwrap().to_string(),
				call["name"].clone(),
			)
		},
		Protocol::Gemini => {
			let mut got = String::new();
			let mut tool = Value::Null;
			for part in out["candidates"][0]["content"]["parts"].as_array().unwrap() {
				if let Some(text) = part["text"].as_str() {
					got.push_str(text);
				}
				if let Some(call) = part.get("functionCall") {
					tool = call["name"].clone();
				}
			}
			(got, tool)
		},
	};
	assert_eq!(got, text, "{name}: text");
	assert_eq!(tool, "read_file", "{name}: tool call");
}

/// A client's streamed reply must carry `text` and the `read_file` call.
pub fn check_stream(name: &str, client: Protocol, text: &str, out: &[u8]) {
	let mut got = String::new();
	let mut tool = None;
	for event in sse_data(out) {
		match client {
			Protocol::Messages => match event["type"].as_str() {
				Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
					got.push_str(event["delta"]["text"].as_str().unwrap())
				},
				Some("content_block_start") if event["content_block"]["type"] == "tool_use" => {
					tool = Some(event["content_block"]["name"].clone())
				},
				_ => {},
			},
			Protocol::Chat => {
				let delta = &event["choices"][0]["delta"];
				if let Some(content) = delta["content"].as_str() {
					got.push_str(content);
				}
				if let Some(name) = delta["tool_calls"][0]["function"].get("name") {
					tool = Some(name.clone());
				}
			},
			Protocol::Responses => match event["type"].as_str() {
				Some("response.output_text.delta") => got.push_str(event["delta"].as_str().unwrap()),
				Some("response.output_item.added") if event["item"]["type"] == "function_call" => {
					tool = Some(event["item"]["name"].clone())
				},
				_ => {},
			},
			Protocol::Gemini => {
				for part in event["candidates"][0]["content"]["parts"]
					.as_array()
					.into_iter()
					.flatten()
				{
					if let Some(text) = part["text"].as_str() {
						got.push_str(text);
					}
					if let Some(call) = part.get("functionCall") {
						tool = Some(call["name"].clone());
					}
				}
			},
		}
	}
	assert_eq!(
		got,
		text,
		"{name}: text in {}",
		String::from_utf8_lossy(&out[..out.len().min(3000)])
	);
	assert_eq!(tool, Some(json!("read_file")), "{name}: tool call");
}

/// The `data` of each JSON event; `[DONE]` and comments are skipped.
pub fn sse_data(stream: &[u8]) -> Vec<Value> {
	std::str::from_utf8(stream)
		.unwrap()
		.replace("\r\n", "\n")
		.split("\n\n")
		.filter_map(|event| {
			let data: Vec<&str> = event
				.lines()
				.filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
				.collect();
			let data = data.join("\n");
			(!data.is_empty() && data != "[DONE]").then(|| serde_json::from_str(&data).unwrap())
		})
		.collect()
}

// --- Requests ---

const SYSTEM: &str = "You are a coding agent working in a large Rust repository. Read files before \
changing them, keep edits small, and explain what you changed. Prefer the existing helpers over new \
abstractions. When a command fails, read its output before trying again. Quote paths that contain \
spaces, and never run commands that delete data without asking. Answer in the user's language; \
for example, reply in 日本語 when asked in Japanese. Use \"double quotes\" for strings in examples \
and C:\\paths\\like\\this only on Windows.\n\nThe repository uses tabs for indentation, rustfmt \
for formatting and nextest for tests. Benchmarks use divan. Commit only when asked.";

const QUESTION: &str = "The gateway returns a 400 for some gzip-compressed Messages requests. \
Find where the request body is decoded and fix it so compressed bodies are read correctly.";

/// Lines in each tool result. Each line names the turn's module once.
pub const FILE_LINES_PER_TURN: usize = 64;

/// A source file as a tool returns it: code with quotes, escapes, tabs and some non-ASCII.
fn file_text(turn: usize, lines: usize, escaped: bool) -> String {
	let mut text = format!("FILE-{turn}\n");
	for line in 0..lines {
		let row = match line % 4 {
			0 => format!("{line:>4}\tfn read_{line}(path: &str) -> Result<Bytes> {{ // module_{turn}\n"),
			1 => format!(
				"{line:>4}\t\tlet name = \"module_{turn}\\\\{line}.rs\"; // “quoted” naïve 日本語\n"
			),
			2 => format!(
				"{line:>4}\t\tif body.len() > LIMIT {{ return Err(Error::TooLarge(\"module_{turn}\")); }}\n"
			),
			_ => format!("{line:>4}\t}} // end of module_{turn} read path, see the comment above\n"),
		};
		text.push_str(&row);
	}
	if escaped {
		text.push_str(r#"Escapes: \"quoted\", C:\\repo\\src, tab\t, newline\n, 日本語.\n"#);
	}
	text
}

fn step_text(turn: usize) -> String {
	format!(
		"STEP-{turn}: The request path reads the body in another module, so I'll open it next and \
check how the encoding header is handled."
	)
}

fn tool_arguments(turn: usize) -> Value {
	json!({"path": format!("crates/agentgateway/src/module_{turn}.rs"), "offset": 0, "limit": 400})
}

struct ToolSpec {
	name: &'static str,
	description: &'static str,
	parameters: Value,
}

fn tools(schema_heavy: bool) -> Vec<ToolSpec> {
	let mut tools = vec![
		ToolSpec {
			name: "read_file",
			description: "Read a file from the repository. Returns numbered lines. Use offset and \
limit for large files.",
			parameters: json!({
				"type": "object",
				"properties": {
					"path": {"type": "string", "description": "Path relative to the repository root."},
					"offset": {"type": "integer", "description": "First line to read, from 0."},
					"limit": {"type": "integer", "description": "Most lines to read."}
				},
				"required": ["path"]
			}),
		},
		ToolSpec {
			name: "write_file",
			description: "Replace a file's contents. Read the file first so nothing is lost.",
			parameters: json!({
				"type": "object",
				"properties": {
					"path": {"type": "string", "description": "Path relative to the repository root."},
					"content": {"type": "string", "description": "The complete new contents."}
				},
				"required": ["path", "content"]
			}),
		},
		ToolSpec {
			name: "run_command",
			description: "Run a shell command in the repository and return its output and exit \
status.",
			parameters: json!({
				"type": "object",
				"properties": {
					"command": {"type": "string", "description": "The command line."},
					"timeout_ms": {"type": "integer", "description": "Stop the command after this long."},
					"cwd": {"type": "string", "description": "Working directory, relative to the root."}
				},
				"required": ["command"]
			}),
		},
		ToolSpec {
			name: "search",
			description: "Search file contents with a regular expression and return matching \
lines with their paths.",
			parameters: json!({
				"type": "object",
				"properties": {
					"pattern": {"type": "string", "description": "A regular expression."},
					"path": {"type": "string", "description": "Directory to search."},
					"glob": {"type": "string", "description": "Only files matching this glob."},
					"case_sensitive": {"type": "boolean"}
				},
				"required": ["pattern"]
			}),
		},
	];
	if schema_heavy {
		tools[0].parameters["properties"]["options"] = json!({
			"type": "object",
			"properties": {
				"encoding": {"type": "string", "enum": ["utf-8", "utf-16", "latin-1"]},
				"filters": {"type": "array", "items": {"type": "object", "properties": {
					"pattern": {"type": "string"}, "case_sensitive": {"type": "boolean"}
				}, "required": ["pattern"], "additionalProperties": false}}
			},
			"additionalProperties": false
		});
	}
	tools
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RequestWorkload {
	Standard,
	EscapedText,
	SchemaHeavy,
	ManySmallTurns,
	ReasoningSettings,
}

impl fmt::Display for RequestWorkload {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::Standard => "standard",
			Self::EscapedText => "escaped-text",
			Self::SchemaHeavy => "schema-heavy",
			Self::ManySmallTurns => "many-small-turns",
			Self::ReasoningSettings => "reasoning-settings",
		})
	}
}

pub struct Conversation {
	pub body: Vec<u8>,
	/// Tool turns: an assistant step with a call, then its result.
	pub turns: usize,
}

/// The largest agent conversation in `protocol` that fits in `size` bytes, with at least the
/// question. Gemini names the model in the URL, so its body has none.
pub fn conversation(protocol: Protocol, model: &str, size: usize) -> Conversation {
	conversation_workload(protocol, model, size, RequestWorkload::Standard)
}

pub fn conversation_workload(
	protocol: Protocol,
	model: &str,
	size: usize,
	workload: RequestWorkload,
) -> Conversation {
	let base = conversation_with(protocol, model, 0, workload).len();
	let per_turn = conversation_with(protocol, model, 1, workload).len() - base;
	let mut turns = size.saturating_sub(base) / per_turn;
	loop {
		let body = conversation_with(protocol, model, turns, workload);
		if body.len() <= size || turns == 0 {
			return Conversation { body, turns };
		}
		turns -= 1;
	}
}

fn conversation_with(
	protocol: Protocol,
	model: &str,
	turns: usize,
	workload: RequestWorkload,
) -> Vec<u8> {
	let tools = tools(workload == RequestWorkload::SchemaHeavy);
	let lines = if workload == RequestWorkload::ManySmallTurns {
		4
	} else {
		FILE_LINES_PER_TURN
	};
	let escaped = workload == RequestWorkload::EscapedText;
	let mut body = match protocol {
		Protocol::Messages => {
			let mut messages = vec![json!({"role": "user", "content": QUESTION})];
			for turn in 0..turns {
				let id = format!("toolu_{turn:06}");
				messages.push(json!({"role": "assistant", "content": [
					{"type": "text", "text": step_text(turn)},
					{"type": "tool_use", "id": id, "name": "read_file", "input": tool_arguments(turn)}
				]}));
				messages.push(json!({"role": "user", "content": [
					{"type": "tool_result", "tool_use_id": id, "content": file_text(turn, lines, escaped)}
				]}));
			}
			let tools: Vec<_> = tools
				.into_iter()
				.map(|tool| json!({"name": tool.name, "description": tool.description, "input_schema": tool.parameters}))
				.collect();
			json!({"model": model, "max_tokens": 8192, "system": SYSTEM, "tools": tools, "messages": messages})
		},
		Protocol::Chat => {
			let mut messages = vec![
				json!({"role": "system", "content": SYSTEM}),
				json!({"role": "user", "content": QUESTION}),
			];
			for turn in 0..turns {
				let id = format!("call_{turn:06}");
				messages.push(json!({"role": "assistant", "content": step_text(turn), "tool_calls": [
					{"id": id, "type": "function", "function": {"name": "read_file", "arguments": tool_arguments(turn).to_string()}}
				]}));
				messages.push(
					json!({"role": "tool", "tool_call_id": id, "content": file_text(turn, lines, escaped)}),
				);
			}
			let tools: Vec<_> = tools
				.into_iter()
				.map(|tool| json!({"type": "function", "function": {"name": tool.name, "description": tool.description, "parameters": tool.parameters}}))
				.collect();
			json!({"model": model, "max_tokens": 8192, "tools": tools, "messages": messages})
		},
		Protocol::Responses => {
			let mut input = vec![json!({"role": "user", "content": QUESTION})];
			for turn in 0..turns {
				let id = format!("call_{turn:06}");
				input.push(json!({"role": "assistant", "content": step_text(turn)}));
				input.push(json!({"type": "function_call", "call_id": id, "name": "read_file", "arguments": tool_arguments(turn).to_string()}));
				input
					.push(json!({"type": "function_call_output", "call_id": id, "output": file_text(turn, lines, escaped)}));
			}
			let tools: Vec<_> = tools
				.into_iter()
				.map(|tool| json!({"type": "function", "name": tool.name, "description": tool.description, "parameters": tool.parameters}))
				.collect();
			json!({"model": model, "max_output_tokens": 8192, "instructions": SYSTEM, "tools": tools, "input": input})
		},
		Protocol::Gemini => {
			let mut contents = vec![json!({"role": "user", "parts": [{"text": QUESTION}]})];
			for turn in 0..turns {
				contents.push(json!({"role": "model", "parts": [
					{"text": step_text(turn)},
					{"functionCall": {"name": "read_file", "args": tool_arguments(turn)}}
				]}));
				contents.push(json!({"role": "user", "parts": [
					{"functionResponse": {"name": "read_file", "response": {"output": file_text(turn, lines, escaped)}}}
				]}));
			}
			let declarations: Vec<_> = tools
				.into_iter()
				.map(|tool| json!({"name": tool.name, "description": tool.description, "parameters": tool.parameters}))
				.collect();
			json!({
				"systemInstruction": {"parts": [{"text": SYSTEM}]},
				"tools": [{"functionDeclarations": declarations}],
				"contents": contents,
				"generationConfig": {"maxOutputTokens": 8192}
			})
		},
	};
	if workload == RequestWorkload::ReasoningSettings {
		match protocol {
			Protocol::Messages => {
				body["thinking"] = json!({"type": "adaptive", "display": "omitted"});
				body["output_config"] = json!({"effort": "medium"});
			},
			Protocol::Responses => body["reasoning"] = json!({"effort": "medium", "summary": "auto"}),
			Protocol::Chat => body["reasoning_effort"] = json!("medium"),
			Protocol::Gemini => {
				body["generationConfig"]["thinkingConfig"] =
					json!({"thinkingBudget": 2048, "includeThoughts": true})
			},
		}
	}
	serde_json::to_vec(&body).unwrap()
}

// --- Replies ---

/// Prose of about `size` bytes with quotes, newlines and some non-ASCII.
pub fn reply_text(size: usize) -> String {
	const SENTENCE: &str = "The body is decoded before it is parsed, so a \"gzip\" request now \
reads correctly; naïve clients that send 日本語 text still work.\n";
	SENTENCE.repeat(size.div_ceil(SENTENCE.len()))
}

/// Pieces of about one token, as providers stream them.
fn deltas(text: &str) -> Vec<String> {
	let chars: Vec<char> = text.chars().collect();
	chars
		.chunks(4)
		.map(|chunk| chunk.iter().collect())
		.collect()
}

const CALL_ARGUMENTS: &str =
	r#"{"path":"crates/agentgateway/src/llm/mod.rs","offset":3440,"limit":80}"#;

impl Protocol {
	/// A provider's whole reply: `text`, then a call to `read_file`.
	pub fn buffered(self, text: &str) -> Vec<u8> {
		let body = match self {
			Self::Messages => json!({
				"id": "msg_01", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
				"content": [
					{"type": "text", "text": text},
					{"type": "tool_use", "id": "toolu_01", "name": "read_file", "input": serde_json::from_str::<Value>(CALL_ARGUMENTS).unwrap()}
				],
				"stop_reason": "tool_use", "stop_sequence": null,
				"usage": {"input_tokens": 1200, "output_tokens": 300}
			}),
			Self::Responses => json!({
				"id": "resp_01", "object": "response", "created_at": 1_700_000_000, "model": "gpt-5",
				"status": "completed",
				"output": [
					{"type": "message", "id": "msg_01", "role": "assistant", "status": "completed",
						"content": [{"type": "output_text", "text": text, "annotations": []}]},
					{"type": "function_call", "id": "fc_01", "call_id": "call_01", "name": "read_file",
						"arguments": CALL_ARGUMENTS, "status": "completed"}
				],
				"usage": {"input_tokens": 1200, "output_tokens": 300, "total_tokens": 1500,
					"input_tokens_details": {"cached_tokens": 0}, "output_tokens_details": {"reasoning_tokens": 0}}
			}),
			Self::Chat => json!({
				"id": "chatcmpl-01", "object": "chat.completion", "created": 1_700_000_000, "model": "gpt-4.1",
				"choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
					"role": "assistant", "content": text,
					"tool_calls": [{"id": "call_01", "type": "function", "function": {"name": "read_file", "arguments": CALL_ARGUMENTS}}]
				}}],
				"usage": {"prompt_tokens": 1200, "completion_tokens": 300, "total_tokens": 1500}
			}),
			Self::Gemini => json!({
				"candidates": [{"index": 0, "finishReason": "STOP", "content": {"role": "model", "parts": [
					{"text": text},
					{"functionCall": {"name": "read_file", "args": serde_json::from_str::<Value>(CALL_ARGUMENTS).unwrap()}}
				]}}],
				"usageMetadata": {"promptTokenCount": 1200, "candidatesTokenCount": 300, "totalTokenCount": 1500},
				"modelVersion": "gemini-2.5-pro", "responseId": "resp_01"
			}),
		};
		serde_json::to_vec(&body).unwrap()
	}

	/// A provider's streamed reply, one SSE event per element.
	pub fn stream(self, text: &str) -> Vec<Vec<u8>> {
		let deltas = deltas(text);
		let (args_a, args_b) = CALL_ARGUMENTS.split_at(CALL_ARGUMENTS.len() / 2);
		let mut events = Vec::new();
		match self {
			Self::Messages => {
				events.push(json!({"type": "message_start", "message": {"id": "msg_01", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5", "content": [], "stop_reason": null, "usage": {"input_tokens": 1200, "output_tokens": 1}}}));
				events.push(json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}));
				for delta in &deltas {
					events.push(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": delta}}));
				}
				events.push(json!({"type": "content_block_stop", "index": 0}));
				events.push(json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "toolu_01", "name": "read_file", "input": {}}}));
				for part in [args_a, args_b] {
					events.push(json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": part}}));
				}
				events.push(json!({"type": "content_block_stop", "index": 1}));
				events.push(json!({"type": "message_delta", "delta": {"stop_reason": "tool_use", "stop_sequence": null}, "usage": {"output_tokens": 300}}));
				events.push(json!({"type": "message_stop"}));
			},
			Self::Responses => {
				let message = json!({"type": "message", "id": "msg_01", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": text, "annotations": []}]});
				let call = json!({"type": "function_call", "id": "fc_01", "call_id": "call_01", "name": "read_file", "arguments": CALL_ARGUMENTS, "status": "completed"});
				events.push(json!({"type": "response.created", "response": {"id": "resp_01", "object": "response", "created_at": 1_700_000_000, "model": "gpt-5", "status": "in_progress", "output": []}}));
				events.push(json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "message", "id": "msg_01", "role": "assistant", "status": "in_progress", "content": []}}));
				events.push(json!({"type": "response.content_part.added", "item_id": "msg_01", "output_index": 0, "content_index": 0, "part": {"type": "output_text", "text": "", "annotations": []}}));
				for delta in &deltas {
					events.push(json!({"type": "response.output_text.delta", "item_id": "msg_01", "output_index": 0, "content_index": 0, "delta": delta}));
				}
				events.push(json!({"type": "response.output_text.done", "item_id": "msg_01", "output_index": 0, "content_index": 0, "text": text}));
				events.push(json!({"type": "response.content_part.done", "item_id": "msg_01", "output_index": 0, "content_index": 0, "part": {"type": "output_text", "text": text, "annotations": []}}));
				events
					.push(json!({"type": "response.output_item.done", "output_index": 0, "item": message}));
				events.push(json!({"type": "response.output_item.added", "output_index": 1, "item": {"type": "function_call", "id": "fc_01", "call_id": "call_01", "name": "read_file", "arguments": "", "status": "in_progress"}}));
				for part in [args_a, args_b] {
					events.push(json!({"type": "response.function_call_arguments.delta", "item_id": "fc_01", "output_index": 1, "delta": part}));
				}
				events.push(json!({"type": "response.function_call_arguments.done", "item_id": "fc_01", "output_index": 1, "arguments": CALL_ARGUMENTS}));
				events.push(json!({"type": "response.output_item.done", "output_index": 1, "item": call}));
				events.push(json!({"type": "response.completed", "response": {"id": "resp_01", "object": "response", "created_at": 1_700_000_000, "model": "gpt-5", "status": "completed", "output": [message, call], "usage": {"input_tokens": 1200, "output_tokens": 300, "total_tokens": 1500}}}));
			},
			Self::Chat => {
				let chunk = |delta: Value, finish: Value| json!({"id": "chatcmpl-01", "object": "chat.completion.chunk", "created": 1_700_000_000, "model": "gpt-4.1", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]});
				events.push(chunk(
					json!({"role": "assistant", "content": ""}),
					Value::Null,
				));
				for delta in &deltas {
					events.push(chunk(json!({"content": delta}), Value::Null));
				}
				events.push(chunk(json!({"tool_calls": [{"index": 0, "id": "call_01", "type": "function", "function": {"name": "read_file", "arguments": ""}}]}), Value::Null));
				for part in [args_a, args_b] {
					events.push(chunk(
						json!({"tool_calls": [{"index": 0, "function": {"arguments": part}}]}),
						Value::Null,
					));
				}
				events.push(chunk(json!({}), json!("tool_calls")));
				events.push(json!({"id": "chatcmpl-01", "object": "chat.completion.chunk", "created": 1_700_000_000, "model": "gpt-4.1", "choices": [], "usage": {"prompt_tokens": 1200, "completion_tokens": 300, "total_tokens": 1500}}));
			},
			Self::Gemini => {
				for delta in &deltas {
					events.push(json!({"candidates": [{"index": 0, "content": {"role": "model", "parts": [{"text": delta}]}}], "modelVersion": "gemini-2.5-pro", "responseId": "resp_01"}));
				}
				events.push(json!({"candidates": [{"index": 0, "finishReason": "STOP", "content": {"role": "model", "parts": [{"functionCall": {"name": "read_file", "args": serde_json::from_str::<Value>(CALL_ARGUMENTS).unwrap()}}]}}],
					"usageMetadata": {"promptTokenCount": 1200, "candidatesTokenCount": 300, "totalTokenCount": 1500}, "modelVersion": "gemini-2.5-pro", "responseId": "resp_01"}));
			},
		}
		if self == Self::Responses {
			for (at, event) in events.iter_mut().enumerate() {
				event["sequence_number"] = json!(at);
			}
		}
		// Anthropic and OpenAI Responses name each event, as they send them.
		let named = matches!(self, Self::Messages | Self::Responses);
		let mut events: Vec<Vec<u8>> = events
			.into_iter()
			.map(|event| {
				match event["type"].as_str() {
					Some(name) if named => format!("event: {name}\ndata: {event}\n\n"),
					// As Google ends each event.
					_ if self == Self::Gemini => format!("data: {event}\r\n\r\n"),
					_ => format!("data: {event}\n\n"),
				}
				.into_bytes()
			})
			.collect();
		if self == Self::Chat {
			events.push(b"data: [DONE]\n\n".to_vec());
		}
		events
	}
}
