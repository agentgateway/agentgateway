//! A reply for a Chat Completions client, streamed as `chat.completion.chunk` events or
//! whole as one `chat.completion`. One state builds both.
//!
//! Nothing goes out until the first content, so an early failure can still be an HTTP error.
//! After that a failure is an `error` event, with no `[DONE]` after it. Parts can overlap:
//! only the first open part streams, and later ones wait for their final value. A tool call
//! goes out whole at its final value, so a client never receives half a call, with the
//! reasoning before it encoded into its id (see `encode_call_id`). Reasoning text is written as
//! `reasoning_content`, as engines that show reasoning write it.

use std::collections::VecDeque;
use std::fmt::Write as _;

use serde::Serialize;
use serde_json::value::RawValue;

use super::{CarriedReasoning, encode_call_id};
use crate::carrier::StateScope;
use crate::ir::events::{
	ErrorKind, Event, Final, JsonStr, OpenText, Outcome, PartId, PartKind, StopReason, Usage,
	decoded_len,
};
use crate::json::{is_strict_object, push_string};
use crate::protocols::responses::writer::{error_code, message};
use crate::protocols::{HttpReply, ToolRoute, is_field_name};

pub struct ChatStream<'a> {
	state: ChatState<'a>,
}

impl<'a> ChatStream<'a> {
	/// `max_held_bytes` bounds what waits for an earlier part, and the reasoning kept for
	/// the next call's id. `created` is the client's Unix time.
	#[must_use]
	pub fn new(
		scope: StateScope<'a>,
		routes: &'a [ToolRoute],
		created: u64,
		max_held_bytes: usize,
	) -> Self {
		Self {
			state: ChatState::new(scope, routes, created, max_held_bytes, true),
		}
	}

	/// A last chunk with usage and no choices, for a client that asked with
	/// `stream_options.include_usage`.
	#[must_use]
	pub const fn include_usage(mut self, include: bool) -> Self {
		self.state.include_usage = include;
		self
	}

	pub fn event(&mut self, event: &Event<'_>) {
		self.state.event(event);
	}

	/// Later events are ignored.
	pub fn fail(&mut self, status: u16) {
		self.state.fail("server_error", status);
	}

	/// The SSE output so far. Send it after each provider chunk.
	pub fn take_output(&mut self) -> Vec<u8> {
		std::mem::take(&mut self.state.out)
	}

	#[must_use]
	pub const fn is_ended(&self) -> bool {
		self.state.ended
	}

	/// The HTTP reply for a failure before any output. After output starts this is always
	/// `None`, and a failure travels in the stream. When it's `Some`, `take_output` has
	/// nothing, and the caller sends this instead.
	#[must_use]
	pub fn early_failure(&self) -> Option<HttpReply> {
		match self.state.failure {
			Some((code, status)) if !self.state.started => Some(self.state.failure_reply(code, status)),
			_ => None,
		}
	}
}

pub struct BufferedChat<'a> {
	state: ChatState<'a>,
}

impl<'a> BufferedChat<'a> {
	/// `max_output_bytes` bounds the reply's output.
	#[must_use]
	pub fn new(
		scope: StateScope<'a>,
		routes: &'a [ToolRoute],
		created: u64,
		max_output_bytes: usize,
	) -> Self {
		Self {
			state: ChatState::new(scope, routes, created, max_output_bytes, false),
		}
	}

	pub fn event(&mut self, event: &Event<'_>) {
		self.state.event(event);
	}

	/// Later events are ignored.
	pub fn fail(&mut self, status: u16) {
		self.state.fail("server_error", status);
	}

	/// A reply that never ended is a 500, so a caller whose input stopped early should say
	/// why with `fail`.
	#[must_use]
	pub fn into_reply(self) -> HttpReply {
		let state = self.state;
		match (state.failure, state.whole.finish_reason) {
			(Some((code, status)), _) => state.failure_reply(code, status),
			(None, Some(reason)) => HttpReply {
				status: 200,
				body: state.completion(reason).into_bytes(),
			},
			(None, None) => error_body_reply("server_error", 500),
		}
	}
}

/// A provider's HTTP error as a Chat Completions client's. An error with no known meaning is
/// still a server failure, so a client retries it.
#[must_use]
pub fn error_reply(status: u16, kind: ErrorKind) -> HttpReply {
	let (code, mapped) = error_code(kind);
	let status = if kind == ErrorKind::Unknown {
		status.max(500)
	} else {
		mapped
	};
	error_body_reply(code, status)
}

/// `error_reply`, naming the field a provider refused. A field name is the client's own
/// vocabulary, where the provider's words aren't.
#[must_use]
pub fn error_reply_naming(status: u16, kind: ErrorKind, field: Option<&str>) -> HttpReply {
	let reply = error_reply(status, kind);
	let (code, _) = error_code(kind);
	match field.filter(|field| code == "invalid_prompt" && is_field_name(field)) {
		Some(field) => HttpReply {
			status: reply.status,
			body: error_json(
				code,
				reply.status,
				&format!("The provider refused `{field}`."),
			)
			.into_bytes(),
		},
		None => reply,
	}
}

struct ChatState<'a> {
	scope: StateScope<'a>,
	routes: &'a [ToolRoute],
	created: u64,
	max_held_bytes: usize,
	streaming: bool,
	include_usage: bool,
	id: String,
	model: String,
	usage: Usage,
	started: bool,
	ended: bool,
	failure: Option<(&'static str, u16)>,
	/// What the client is told of the failure, in place of its code's fixed text.
	client_message: Option<String>,
	/// Parts not yet written, in start order. Only the first can stream.
	parts: VecDeque<OpenPart>,
	/// Provider reasoning items since the last call, for the next call's id.
	reasoning: Vec<CarriedReasoning>,
	/// Decoded answer bytes written before the current point in the reply.
	text_bytes: usize,
	calls: usize,
	dropped_call: bool,
	held_bytes: usize,
	/// Streamed output not yet taken.
	out: Vec<u8>,
	/// A reply that isn't streamed, built up as it would have streamed.
	whole: WholeMessage,
}

#[derive(Default)]
struct WholeMessage {
	/// Answer text as JSON string contents, including escapes but excluding quotes.
	content: Option<String>,
	refusal: Option<String>,
	reasoning: Option<String>,
	tool_calls: Vec<String>,
	finish_reason: Option<&'static str>,
}

struct OpenPart {
	id: PartId,
	kind: Kind,
	/// Answer text from its deltas, kept when it can't stream as it comes.
	text: Option<OpenText>,
	kept_bytes: usize,
	/// A call's id and name, until its final value.
	call: Option<(String, String)>,
	/// The final value of a part still waiting for an earlier one.
	held: Option<Done>,
	sent_delta: bool,
	/// A delta came while this part waited, so it goes out whole at the end.
	missed_delta: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
	Text,
	Refusal,
	Reasoning,
	Call,
}

/// A part's final value, owned so it can wait.
enum Done {
	/// The text from deltas that didn't stream, as a JSON string.
	Text(Option<String>),
	Refusal(String),
	Reasoning {
		item: String,
		text: Option<String>,
	},
	Call {
		call_id: String,
		name: String,
		arguments: Box<RawValue>,
		complete: bool,
	},
}

impl Done {
	fn size(&self) -> usize {
		match self {
			Self::Text(text) => text.as_ref().map_or(0, String::len),
			Self::Refusal(text) => text.len(),
			Self::Reasoning { item, text } => item.len() + text.as_ref().map_or(0, String::len),
			Self::Call {
				call_id,
				name,
				arguments,
				..
			} => call_id.len() + name.len() + arguments.get().len(),
		}
	}
}

impl<'a> ChatState<'a> {
	fn new(
		scope: StateScope<'a>,
		routes: &'a [ToolRoute],
		created: u64,
		max_held_bytes: usize,
		streaming: bool,
	) -> Self {
		Self {
			scope,
			routes,
			created,
			max_held_bytes,
			streaming,
			include_usage: false,
			id: String::new(),
			model: String::new(),
			usage: Usage::default(),
			started: false,
			ended: false,
			failure: None,
			client_message: None,
			parts: VecDeque::new(),
			reasoning: Vec::new(),
			text_bytes: 0,
			calls: 0,
			dropped_call: false,
			held_bytes: 0,
			out: Vec::new(),
			whole: WholeMessage::default(),
		}
	}

	fn event(&mut self, event: &Event<'_>) {
		if self.ended {
			return;
		}
		match *event {
			Event::ResponseStart { id, model } => {
				self.id = format!("chatcmpl-{id}");
				self.model = model.to_string();
			},
			Event::PartStart { part, kind } => self.part_start(part, kind),
			Event::Delta { part, text } => self.delta(part, text),
			Event::PartEnd { part, value } => self.part_end(part, value),
			Event::Usage(usage) => self.usage = usage,
			Event::Finish(outcome) => self.finish(outcome),
			Event::Error(error) => {
				let (code, status) = error_code(error.kind);
				self.fail_saying(code, status, error.client_message);
			},
			// Chat Completions has no place for a citation without its position in the text.
			Event::Citation { .. }
			| Event::MessageEnd { .. }
			| Event::ServiceTier(_)
			| Event::StopSequence(_) => {},
		}
	}

	fn fail(&mut self, code: &'static str, status: u16) {
		self.fail_saying(code, status, None);
	}

	/// `message` replaces the code's fixed text.
	fn fail_saying(&mut self, code: &'static str, status: u16, message: Option<&str>) {
		if self.ended {
			return;
		}
		self.ended = true;
		self.parts.clear();
		self.reasoning.clear();
		self.held_bytes = 0;
		self.failure = Some((code, status));
		self.client_message = message.map(str::to_string);
		if self.streaming && self.started {
			let error = error_json(code, status, self.failure_message(code));
			self.out.extend_from_slice(b"data: ");
			self.out.extend_from_slice(error.as_bytes());
			self.out.extend_from_slice(b"\n\n");
		}
	}

	fn failure_reply(&self, code: &'static str, status: u16) -> HttpReply {
		HttpReply {
			status,
			body: error_json(code, status, self.failure_message(code)).into_bytes(),
		}
	}

	fn failure_message(&self, code: &'static str) -> &str {
		self
			.client_message
			.as_deref()
			.unwrap_or_else(|| message(code))
	}

	fn part_start(&mut self, id: PartId, kind: PartKind<'_>) {
		let (kind, call) = match kind {
			PartKind::Text => (Kind::Text, None),
			PartKind::Refusal => (Kind::Refusal, None),
			PartKind::Reasoning => (Kind::Reasoning, None),
			PartKind::ToolCall { call_id, name } => {
				(Kind::Call, Some((call_id.to_string(), name.to_string())))
			},
		};
		let kept_bytes = call.as_ref().map_or(0, |(id, name)| id.len() + name.len());
		self.held_bytes += kept_bytes;
		self.parts.push_back(OpenPart {
			id,
			kind,
			text: None,
			kept_bytes,
			call,
			held: None,
			sent_delta: false,
			missed_delta: false,
		});
		self.check_size();
	}

	fn delta(&mut self, id: PartId, text: JsonStr<'_>) {
		let Some(at) = self.parts.iter().position(|part| part.id == id) else {
			self.fail("server_error", 500);
			return;
		};
		let part = &mut self.parts[at];
		match part.kind {
			Kind::Call => {},
			Kind::Text | Kind::Refusal | Kind::Reasoning if at == 0 && !part.missed_delta => {
				part.sent_delta = true;
				let kind = part.kind;
				self.write_text(kind, text.raw());
			},
			// A refusal or reasoning part's final value has all its text.
			Kind::Refusal | Kind::Reasoning => part.missed_delta = true,
			Kind::Text => {
				part.missed_delta = true;
				let kept = part.text.get_or_insert_with(OpenText::default);
				let before = kept.len();
				kept.push_json(text);
				let added = kept.len() - before;
				part.kept_bytes += added;
				self.held_bytes += added;
				self.check_size();
			},
		}
	}

	fn part_end(&mut self, id: PartId, value: Final<'_>) {
		let Some(at) = self.parts.iter().position(|part| part.id == id) else {
			self.fail("server_error", 500);
			return;
		};
		let part = &mut self.parts[at];
		let done = match (part.kind, value) {
			(Kind::Text, Final::Text) => Done::Text(part.text.take().map(OpenText::finish)),
			(Kind::Refusal, Final::Refusal(text)) => Done::Refusal(text.raw().to_string()),
			(Kind::Reasoning, Final::Reasoning { item, text, .. }) => Done::Reasoning {
				item: item.get().to_string(),
				text: text.map(|text| text.raw().to_string()),
			},
			(Kind::Call, Final::ToolCall(arguments)) => {
				let Some((call_id, name)) = part.call.take() else {
					unreachable!("a call part keeps its identity until it ends");
				};
				Done::Call {
					call_id,
					name,
					arguments: arguments.raw_value().to_owned(),
					complete: arguments.is_complete(),
				}
			},
			_ => {
				self.fail("server_error", 500);
				return;
			},
		};
		// What the part kept now lives in its final value.
		self.held_bytes -= std::mem::take(&mut part.kept_bytes);
		self.held_bytes += done.size();
		self.parts[at].held = Some(done);
		self.check_size();
		while !self.ended && self.parts.front().is_some_and(|part| part.held.is_some()) {
			self.write_front();
		}
	}

	fn write_front(&mut self) {
		let Some(mut part) = self.parts.pop_front() else {
			return;
		};
		let Some(done) = part.held.take() else {
			unreachable!("only a finished part is written");
		};
		self.held_bytes -= done.size();
		match done {
			Done::Text(text) => {
				if let Some(text) = text.filter(|text| text != r#""""#) {
					self.write_text(Kind::Text, &text);
				}
			},
			Done::Refusal(text) => {
				if !part.sent_delta {
					self.write_text(Kind::Refusal, &text);
				}
			},
			Done::Reasoning { item, text } => {
				let text = text.filter(|text| !part.sent_delta && text != r#""""#);
				if let Some(text) = text {
					self.write_text(Kind::Reasoning, &text);
				}
				self.held_bytes += item.len();
				self.reasoning.push(CarriedReasoning {
					text_offset: self.text_bytes,
					item,
				});
			},
			Done::Call {
				call_id,
				name,
				arguments,
				complete,
			} => self.write_call(&call_id, &name, &arguments, complete),
		}
		self.check_size();
	}

	/// A call's arguments must be a JSON object, or nothing for a call that takes none.
	fn write_call(&mut self, call_id: &str, name: &str, arguments: &RawValue, complete: bool) {
		let reasoning = std::mem::take(&mut self.reasoning);
		self.held_bytes -= reasoning
			.iter()
			.map(|reasoning| reasoning.item.len())
			.sum::<usize>();
		if !complete {
			self.dropped_call = true;
			return;
		}
		let text: Option<String> = serde_json::from_str(arguments.get()).ok();
		let arguments = match text.as_deref() {
			Some("") => serde_json::from_str(r#""{}""#)
				.unwrap_or_else(|error| unreachable!("empty call arguments are JSON: {error}")),
			Some(text)
				if serde_json::from_str::<&serde_json::value::RawValue>(text)
					.is_ok_and(is_strict_object) =>
			{
				arguments
			},
			_ => {
				self.fail("server_error", 500);
				return;
			},
		};
		let name = self
			.routes
			.iter()
			.find(|route| route.flat == name)
			.map_or(name, |route| route.name.as_str());
		let id = encode_call_id(self.scope, call_id, &reasoning);
		let index = self.calls;
		self.calls += 1;
		let call = ToolCall {
			index: self.streaming.then_some(index),
			id: &id,
			kind: "function",
			function: FunctionCall { name, arguments },
		};
		let encoded = if self.streaming {
			serde_json::to_string(&ToolCalls { tool_calls: [call] })
		} else {
			serde_json::to_string(&call)
		}
		.unwrap_or_else(|error| unreachable!("a tool call serializes: {error}"));
		if self.streaming {
			self.chunk(&encoded, None);
		} else {
			self.held_bytes += encoded.len();
			self.whole.tool_calls.push(encoded);
		}
	}

	/// `text` is a JSON string, escapes included.
	fn write_text(&mut self, kind: Kind, text: &str) {
		if kind == Kind::Text {
			self.text_bytes += decoded_len(text);
		}
		let field = match kind {
			Kind::Text => "content",
			Kind::Refusal => "refusal",
			Kind::Reasoning => "reasoning_content",
			Kind::Call => unreachable!("a call isn't text"),
		};
		if self.streaming {
			self.chunk(&format!(r#"{{"{field}":{text}}}"#), None);
			return;
		}
		let whole = match kind {
			Kind::Text => &mut self.whole.content,
			Kind::Refusal => &mut self.whole.refusal,
			_ => &mut self.whole.reasoning,
		};
		let inside = &text[1..text.len() - 1];
		whole.get_or_insert_with(String::new).push_str(inside);
		self.held_bytes += inside.len();
	}

	fn finish(&mut self, outcome: Outcome<'_>) {
		if !self.parts.is_empty() {
			self.fail("server_error", 500);
			return;
		}
		let reason = match outcome {
			Outcome::Completed if self.dropped_call => {
				self.fail("server_error", 500);
				return;
			},
			Outcome::Completed if self.calls > 0 => "tool_calls",
			Outcome::Completed => "stop",
			Outcome::Incomplete { reason, .. } => match reason {
				StopReason::MaxTokens => "length",
				StopReason::ContentFilter => "content_filter",
				StopReason::ContextWindow => {
					self.fail("context_length_exceeded", 400);
					return;
				},
				StopReason::Other => {
					self.fail("server_error", 500);
					return;
				},
			},
		};
		if self.streaming {
			self.chunk("{}", Some(reason));
			if self.include_usage {
				let usage = format!(
					r#"{},"choices":[],"usage":{}}}"#,
					self.chunk_head(),
					usage_json(self.usage)
				);
				self.send(&usage);
			}
			self.out.extend_from_slice(b"data: [DONE]\n\n");
		} else {
			self.whole.finish_reason = Some(reason);
		}
		self.ended = true;
	}

	fn chunk(&mut self, delta: &str, finish_reason: Option<&str>) {
		if !std::mem::replace(&mut self.started, true) {
			self.chunk(r#"{"role":"assistant","content":""}"#, None);
		}
		let mut chunk = self.chunk_head();
		let _ = write!(
			chunk,
			r#","choices":[{{"index":0,"delta":{delta},"finish_reason":"#
		);
		match finish_reason {
			Some(reason) => {
				let _ = write!(chunk, r#""{reason}"}}]}}"#);
			},
			None => chunk.push_str("null}]}"),
		}
		self.send(&chunk);
	}

	fn chunk_head(&self) -> String {
		let mut head = String::from(r#"{"id":"#);
		push_string(&mut head, &self.id);
		let _ = write!(
			head,
			r#","object":"chat.completion.chunk","created":{},"model":"#,
			self.created
		);
		push_string(&mut head, &self.model);
		head
	}

	fn send(&mut self, data: &str) {
		self.out.extend_from_slice(b"data: ");
		self.out.extend_from_slice(data.as_bytes());
		self.out.extend_from_slice(b"\n\n");
	}

	fn check_size(&mut self) {
		if self.held_bytes > self.max_held_bytes {
			self.fail("server_error", 500);
		}
	}

	fn completion(&self, finish_reason: &str) -> String {
		let whole = &self.whole;
		let mut out = String::with_capacity(self.held_bytes + 512);
		out.push_str(r#"{"id":"#);
		push_string(&mut out, &self.id);
		let _ = write!(
			out,
			r#","object":"chat.completion","created":{},"model":"#,
			self.created
		);
		push_string(&mut out, &self.model);
		out.push_str(r#","choices":[{"index":0,"message":{"role":"assistant""#);
		for (field, value) in [("content", &whole.content), ("refusal", &whole.refusal)] {
			match value {
				Some(text) => {
					let _ = write!(out, r#","{field}":"{text}""#);
				},
				None => {
					let _ = write!(out, r#","{field}":null"#);
				},
			}
		}
		if let Some(text) = &whole.reasoning {
			let _ = write!(out, r#","reasoning_content":"{text}""#);
		}
		if !whole.tool_calls.is_empty() {
			out.push_str(r#","tool_calls":["#);
			out.push_str(&whole.tool_calls.join(","));
			out.push(']');
		}
		let _ = write!(
			out,
			r#"}},"logprobs":null,"finish_reason":"{finish_reason}"}}],"usage":{}}}"#,
			usage_json(self.usage)
		);
		out
	}
}

#[derive(Serialize)]
struct ToolCalls<'a> {
	tool_calls: [ToolCall<'a>; 1],
}

#[derive(Serialize)]
struct ToolCall<'a> {
	// Keep the stream index first, where a client reading the delta looks for it.
	#[serde(skip_serializing_if = "Option::is_none")]
	index: Option<usize>,
	id: &'a str,
	#[serde(rename = "type")]
	kind: &'static str,
	function: FunctionCall<'a>,
}

#[derive(Serialize)]
struct FunctionCall<'a> {
	name: &'a str,
	arguments: &'a RawValue,
}

/// Chat Completions requires token counts, so unknown is written as 0 here. Accounting
/// reads the usage fold, where unknown stays unknown.
fn usage_json(usage: Usage) -> String {
	let input = usage.input_tokens.unwrap_or(0);
	let output = usage.output_tokens.unwrap_or(0);
	format!(
		r#"{{"prompt_tokens":{input},"completion_tokens":{output},"total_tokens":{},"prompt_tokens_details":{{"cached_tokens":{}}},"completion_tokens_details":{{"reasoning_tokens":{}}}}}"#,
		usage
			.total_tokens
			.unwrap_or_else(|| input.saturating_add(output)),
		usage.cached_tokens.unwrap_or(0),
		usage.reasoning_tokens.unwrap_or(0),
	)
}

fn error_body_reply(code: &str, status: u16) -> HttpReply {
	HttpReply {
		status,
		body: error_json(code, status, message(code)).into_bytes(),
	}
}

/// Clients read the HTTP status first; `type` follows it, and `code` says which failure.
fn error_json(code: &str, status: u16, text: &str) -> String {
	let kind = match status {
		401 => "authentication_error",
		403 => "permission_error",
		404 => "not_found_error",
		429 if code == "insufficient_quota" => "insufficient_quota",
		429 => "rate_limit_error",
		400..=499 => "invalid_request_error",
		_ => "server_error",
	};
	let mut out = String::from(r#"{"error":{"message":"#);
	push_string(&mut out, text);
	let _ = write!(out, r#","type":"{kind}","param":null,"code":"{code}"}}}}"#);
	out
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
