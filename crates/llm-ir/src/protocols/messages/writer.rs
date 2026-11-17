//! Messages allows one open block at a time, while a provider's parts can overlap,
//! so the sequencer decides what is sent now, what is held and what fails.
//! `SseWriter` and `ReplyWriter` read the same operations, so streamed and
//! buffered replies can't drift apart.
//!
//! Tool calls go out only at their final value, so a client never receives half a
//! call.

use std::collections::VecDeque;
use std::fmt;
use std::io::Write as _;

use serde::Serialize;
use serde_json::value::RawValue;

use super::{ErrorType, Failure, classify, error_body, error_json, json_string};
use crate::carrier::{self, StateScope};
use crate::ir::events::{
	Event, Final, JsonStr, OpenText, Outcome, PartId, PartKind, ProviderError, StopReason, Usage,
};
use crate::protocols::HttpReply;
use crate::sse::{close_event, open_event};

#[derive(Default)]
pub struct SseWriter {
	pub out: Vec<u8>,
	index: usize,
	started: bool,
	early_failure: Option<Failure>,
}

impl SseWriter {
	/// Anthropic sends a failure before `message_start` as an HTTP error, and Claude
	/// Code retries an error event in a stream without streaming. When this is
	/// `Some`, nothing but the error has been written: send it in place of `out`,
	/// before any head.
	#[must_use]
	pub fn early_failure(&self) -> Option<HttpReply> {
		self.early_failure.as_ref().map(|failure| HttpReply {
			status: failure.kind.status(),
			body: error_body(failure),
		})
	}
}

impl Sink for SseWriter {
	fn op(&mut self, op: Op<'_>) {
		match op {
			Op::Start { id, model } => {
				self.started = true;
				self.frame(
					"message_start",
					format_args!(
						r#"{{"type":"message_start","message":{{"id":{},"type":"message","role":"assistant","content":[],"model":{},"stop_reason":null,"stop_sequence":null,"usage":{{"input_tokens":0,"output_tokens":0}}}}}}"#,
						json_string(id),
						json_string(model)
					),
				);
			},
			Op::TextStart => self.block_start(format_args!(r#"{{"type":"text","text":""}}"#)),
			Op::ThinkingStart => self.block_start(format_args!(
				r#"{{"type":"thinking","thinking":"","signature":""}}"#
			)),
			Op::ToolStart { id, name } => {
				self.block_start(format_args!(
					r#"{{"type":"tool_use","id":{},"name":{},"input":{{}}}}"#,
					json_string(id),
					json_string(name)
				));
			},
			Op::Text { raw } => self.delta("text", "text_delta", raw),
			Op::Thinking { raw } => self.delta("thinking", "thinking_delta", raw),
			Op::Arguments { raw } => self.delta("partial_json", "input_json_delta", raw),
			Op::Signature { value } => {
				self.delta("signature", "signature_delta", &json_string(value));
			},
			Op::Citation { url, title } => {
				let citation = Citation::new(url, title);
				let citation = serde_json::to_string(&citation).unwrap_or_default();
				self.delta("citation", "citations_delta", &citation);
			},
			Op::BlockEnd => {
				let index = self.index;
				self.frame(
					"content_block_stop",
					format_args!(r#"{{"type":"content_block_stop","index":{index}}}"#),
				);
				self.index += 1;
			},
			Op::End {
				stop_reason,
				stop_sequence,
				usage,
				service_tier,
			} => {
				let stop_sequence = stop_sequence.map_or_else(|| "null".to_string(), json_string);
				let usage = client_usage(usage, service_tier);
				let usage = serde_json::to_string(&usage).unwrap_or_else(|_| "{}".to_string());
				self.frame(
					"message_delta",
					format_args!(
						r#"{{"type":"message_delta","delta":{{"stop_reason":"{stop_reason}","stop_sequence":{stop_sequence}}},"usage":{usage}}}"#
					),
				);
				self.frame("message_stop", format_args!(r#"{{"type":"message_stop"}}"#));
			},
			Op::Fail(failure) => {
				if !self.started {
					self.early_failure = Some(failure.clone());
				}
				self.frame("error", format_args!("{}", error_json(failure)));
			},
		}
	}
}

impl SseWriter {
	fn frame(&mut self, name: &str, data: fmt::Arguments<'_>) {
		open_event(&mut self.out, name);
		let _ = self.out.write_fmt(data);
		close_event(&mut self.out);
	}

	fn block_start(&mut self, block: fmt::Arguments<'_>) {
		let index = self.index;
		self.frame(
			"content_block_start",
			format_args!(r#"{{"type":"content_block_start","index":{index},"content_block":{block}}}"#),
		);
	}

	fn delta(&mut self, field: &str, delta_type: &str, raw: &str) {
		let index = self.index;
		self.frame(
			"content_block_delta",
			format_args!(
				r#"{{"type":"content_block_delta","index":{index},"delta":{{"type":"{delta_type}","{field}":{raw}}}}}"#
			),
		);
	}
}

/// Messages requires token counts on the wire, so unknown is written as 0
/// here. Accounting reads the usage fold, where unknown stays unknown.
#[derive(Serialize)]
struct ClientUsage {
	input_tokens: u64,
	output_tokens: u64,
	#[serde(skip_serializing_if = "Option::is_none")]
	cache_read_input_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	cache_creation_input_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	service_tier: Option<&'static str>,
}

/// Responses counts cached tokens inside `input_tokens`; Messages doesn't.
fn client_usage(usage: Usage, service_tier: Option<&'static str>) -> ClientUsage {
	let cached = usage
		.cached_tokens
		.unwrap_or(0)
		.saturating_add(usage.cache_write_tokens.unwrap_or(0));
	ClientUsage {
		input_tokens: usage.input_tokens.unwrap_or(0).saturating_sub(cached),
		output_tokens: usage.output_tokens.unwrap_or(0),
		cache_read_input_tokens: usage.cached_tokens,
		cache_creation_input_tokens: usage.cache_write_tokens,
		service_tier,
	}
}

/// Only values Messages understands. Logs keep the provider's raw value.
pub(super) fn client_service_tier(tier: &str) -> Option<&'static str> {
	match tier {
		"default" => Some("standard"),
		"priority" => Some("priority"),
		_ => None,
	}
}

#[derive(Serialize)]
struct Citation {
	#[serde(rename = "type")]
	kind: &'static str,
	url: String,
	title: Option<String>,
	cited_text: &'static str,
	encrypted_index: &'static str,
}

impl Citation {
	fn new(url: &str, title: Option<&str>) -> Self {
		Self {
			kind: "web_search_result_location",
			url: url.to_string(),
			title: title.map(str::to_string),
			cited_text: "",
			encrypted_index: "",
		}
	}
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ReplyBlock {
	Text {
		text: JsonText,
		#[serde(skip_serializing_if = "Vec::is_empty")]
		citations: Vec<Citation>,
	},
	Thinking {
		thinking: JsonText,
		signature: String,
	},
	ToolUse {
		id: String,
		name: String,
		input: Box<RawValue>,
		#[serde(skip)]
		arguments: String,
	},
}

#[derive(Serialize)]
struct Reply {
	id: String,
	#[serde(rename = "type")]
	kind: &'static str,
	role: &'static str,
	content: Vec<ReplyBlock>,
	model: String,
	stop_reason: Option<&'static str>,
	stop_sequence: Option<String>,
	usage: ClientUsage,
}

struct End {
	stop_reason: &'static str,
	stop_sequence: Option<String>,
	usage: Usage,
	service_tier: Option<&'static str>,
}

pub struct ReplyWriter {
	id: String,
	model: String,
	content: Vec<ReplyBlock>,
	end: Option<End>,
	failure: Option<Failure>,
	/// A value the decoder had passed could not be read. Never send a guess.
	broken: bool,
	max_output_bytes: usize,
}

impl Default for ReplyWriter {
	fn default() -> Self {
		Self::new(usize::MAX)
	}
}

impl Sink for ReplyWriter {
	fn op(&mut self, op: Op<'_>) {
		let last = self.content.last_mut();
		match (op, last) {
			(Op::Start { id, model }, _) => {
				self.id = id.to_string();
				self.model = model.to_string();
			},
			(Op::TextStart, _) => self.content.push(ReplyBlock::Text {
				text: JsonText::default(),
				citations: Vec::new(),
			}),
			(Op::ThinkingStart, _) => self.content.push(ReplyBlock::Thinking {
				thinking: JsonText::default(),
				signature: String::new(),
			}),
			(Op::ToolStart { id, name }, _) => self.content.push(ReplyBlock::ToolUse {
				id: id.to_string(),
				name: name.to_string(),
				input: empty_object(),
				arguments: String::new(),
			}),
			(Op::Citation { url, title }, Some(ReplyBlock::Text { citations, .. })) => {
				citations.push(Citation::new(url, title));
			},
			(Op::Text { raw }, Some(ReplyBlock::Text { text, .. })) => {
				self.broken |= !text.push(raw);
			},
			(Op::Thinking { raw }, Some(ReplyBlock::Thinking { thinking, .. })) => {
				self.broken |= !thinking.push(raw);
			},
			(Op::Arguments { raw }, Some(ReplyBlock::ToolUse { arguments, .. })) => {
				self.broken |= !push_raw(arguments, raw);
			},
			(Op::Signature { value }, Some(ReplyBlock::Thinking { signature, .. })) => {
				signature.push_str(value);
			},
			(
				Op::BlockEnd,
				Some(ReplyBlock::ToolUse {
					input, arguments, ..
				}),
			) => {
				if !arguments.is_empty() {
					match RawValue::from_string(std::mem::take(arguments)) {
						Ok(parsed) => *input = parsed,
						Err(_) => self.broken = true,
					}
				}
			},
			(
				Op::End {
					stop_reason,
					stop_sequence,
					usage,
					service_tier,
				},
				_,
			) => {
				self.end = Some(End {
					stop_reason,
					stop_sequence: stop_sequence.map(str::to_string),
					usage,
					service_tier,
				});
			},
			(Op::Fail(failure), _) => self.failure = Some(failure.clone()),
			_ => {},
		}
	}
}

impl ReplyWriter {
	#[must_use]
	pub const fn new(max_output_bytes: usize) -> Self {
		Self {
			id: String::new(),
			model: String::new(),
			content: Vec::new(),
			end: None,
			failure: None,
			broken: false,
			max_output_bytes,
		}
	}

	/// A reply with no ending is a failure.
	#[must_use]
	pub fn finish(self) -> HttpReply {
		let max_output_bytes = self.max_output_bytes;
		let failure = self
			.failure
			.unwrap_or_else(|| Failure::new(ErrorType::Api, "the reply did not finish"));
		let end = self.end.filter(|_| !self.broken);
		let Some(end) = end else {
			return HttpReply {
				status: failure.kind.status(),
				body: error_body(&failure),
			};
		};
		let reply = Reply {
			id: self.id,
			kind: "message",
			role: "assistant",
			content: self.content,
			model: self.model,
			stop_reason: Some(end.stop_reason),
			stop_sequence: end.stop_sequence,
			usage: client_usage(end.usage, end.service_tier),
		};
		match serde_json::to_vec(&reply) {
			Ok(body) if body.len() <= max_output_bytes => HttpReply { status: 200, body },
			Ok(_) => {
				let failure = Failure::new(ErrorType::Api, "the reply exceeded its output limit");
				HttpReply {
					status: failure.kind.status(),
					body: error_body(&failure),
				}
			},
			Err(err) => {
				let failure = Failure::new(ErrorType::Api, err.to_string());
				HttpReply {
					status: failure.kind.status(),
					body: error_body(&failure),
				}
			},
		}
	}
}

fn empty_object() -> Box<RawValue> {
	RawValue::from_string("{}".to_string()).unwrap_or_else(|_| unreachable!())
}

/// Returns false when `raw` isn't a JSON string.
fn push_raw(out: &mut String, raw: &str) -> bool {
	let Ok(text) = serde_json::from_str::<String>(raw) else {
		return false;
	};
	out.push_str(&text);
	true
}

/// A block's text as one JSON string, joined from its pieces as they were
/// escaped, so a long reply is not decoded and escaped again.
struct JsonText(String);

impl Default for JsonText {
	fn default() -> Self {
		Self(String::from(r#""""#))
	}
}

impl JsonText {
	/// Returns false when `raw` isn't a JSON string.
	fn push(&mut self, raw: &str) -> bool {
		let Some(text) = raw.strip_prefix('"').and_then(|raw| raw.strip_suffix('"')) else {
			return false;
		};
		self.0.pop();
		// With the closing quote included, a long first piece is allocated once,
		// rather than doubled for that one byte.
		self.0.reserve(text.len() + 1);
		self.0.push_str(text);
		self.0.push('"');
		true
	}
}

impl Serialize for JsonText {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		// Whole JSON strings join into one, so this fails only on a bad piece.
		let raw: &RawValue = serde_json::from_str(&self.0).map_err(serde::ser::Error::custom)?;
		raw.serialize(serializer)
	}
}

/// Strings marked raw are JSON strings, quotes included, exactly as the provider
/// wrote them.
#[derive(Debug, Clone, Copy)]
pub enum Op<'a> {
	Start {
		id: &'a str,
		model: &'a str,
	},
	TextStart,
	ThinkingStart,
	ToolStart {
		id: &'a str,
		name: &'a str,
	},
	Text {
		raw: &'a str,
	},
	Thinking {
		raw: &'a str,
	},
	Arguments {
		raw: &'a str,
	},
	Signature {
		value: &'a str,
	},
	Citation {
		url: &'a str,
		title: Option<&'a str>,
	},
	BlockEnd,
	End {
		stop_reason: &'static str,
		stop_sequence: Option<&'a str>,
		usage: Usage,
		service_tier: Option<&'static str>,
	},
	Fail(&'a Failure),
}

pub trait Sink {
	fn op(&mut self, op: Op<'_>);
}

enum OpenKind {
	Text { unsent: OpenText },
	Refusal,
	Reasoning,
	Call { call_id: String, name: String },
}

/// Text that has already gone out as deltas isn't kept.
enum Value {
	Text {
		unsent: Option<String>,
		refusal: bool,
	},
	Call {
		call_id: String,
		name: String,
		arguments: String,
		complete: bool,
	},
	Reasoning {
		unsent: Option<String>,
		item: String,
	},
}

struct Part {
	id: PartId,
	kind: OpenKind,
	streaming: bool,
	/// Citations that arrived before this part's block opened.
	citations: Vec<HeldCitation>,
	citation_bytes: usize,
	/// A delta came while this part waited. Its text goes out whole at the end.
	missed_delta: bool,
	value: Option<Value>,
	/// Counts against `max_held_bytes` until the part is sent.
	held_bytes: usize,
}

struct HeldCitation {
	url: String,
	title: Option<String>,
}

struct Completed {
	value: Value,
	held_bytes: usize,
	citations: Vec<HeldCitation>,
}

impl Part {
	fn take_completed(&mut self) -> Option<Completed> {
		Some(Completed {
			value: self.value.take()?,
			held_bytes: self.held_bytes + self.citation_bytes,
			citations: std::mem::take(&mut self.citations),
		})
	}
}

pub struct Sequencer<'a> {
	scope: StateScope<'a>,
	max_held_bytes: usize,
	held_bytes: usize,
	response: Option<Response>,
	started: bool,
	ended: bool,
	/// Parts not fully sent, in start order. Only the first can stream.
	parts: VecDeque<Part>,
	saw_call: bool,
	saw_refusal: bool,
	dropped_call: bool,
	usage: Usage,
	service_tier: Option<&'static str>,
	stop_sequence: Option<String>,
}

struct Response {
	id: String,
	model: String,
}

impl<'a> Sequencer<'a> {
	#[must_use]
	pub fn new(scope: StateScope<'a>, max_held_bytes: usize) -> Self {
		Self {
			scope,
			max_held_bytes,
			held_bytes: 0,
			response: None,
			started: false,
			ended: false,
			parts: VecDeque::new(),
			saw_call: false,
			saw_refusal: false,
			dropped_call: false,
			usage: Usage::default(),
			service_tier: None,
			stop_sequence: None,
		}
	}

	/// Before output starts, a failure can be an HTTP error, and a retry can't
	/// duplicate output.
	#[must_use]
	pub const fn output_started(&self) -> bool {
		self.started
	}

	#[must_use]
	pub const fn is_ended(&self) -> bool {
		self.ended
	}

	pub fn event(&mut self, event: &Event<'_>, sink: &mut dyn Sink) {
		if self.ended {
			return;
		}
		if let Err(failure) = self.apply(event, sink) {
			self.fail(&failure, sink);
		}
	}

	pub fn fail(&mut self, failure: &Failure, sink: &mut dyn Sink) {
		if self.ended {
			return;
		}
		self.ended = true;
		self.parts.clear();
		sink.op(Op::Fail(failure));
	}

	fn apply(&mut self, event: &Event<'_>, sink: &mut dyn Sink) -> Result<(), Failure> {
		match *event {
			Event::ResponseStart { id, model } => {
				self.response = Some(Response {
					id: id.to_string(),
					model: model.to_string(),
				});
			},
			Event::PartStart { part, kind } => self.part_start(part, kind)?,
			Event::Delta { part, text } => self.delta(part, text, sink)?,
			Event::Citation { part, url, title } => self.citation(part, url, title, sink)?,
			Event::PartEnd { part, value } => self.part_end(part, value, sink)?,
			Event::MessageEnd { .. } => {},
			Event::ServiceTier(tier) => self.service_tier = client_service_tier(tier),
			Event::StopSequence(sequence) => self.stop_sequence = Some(sequence.to_string()),
			Event::Usage(usage) => self.usage = usage,
			Event::Finish(outcome) => self.finish(outcome, sink)?,
			Event::Error(error) => return Err(from_provider(error)),
		}
		Ok(())
	}

	fn part_start(&mut self, id: PartId, kind: PartKind<'_>) -> Result<(), Failure> {
		let held_bytes = match kind {
			PartKind::ToolCall { call_id, name } => call_id.len() + name.len(),
			_ => 0,
		};
		if held_bytes > self.max_held_bytes.saturating_sub(self.held_bytes) {
			return Err(Failure::new(
				ErrorType::Api,
				"too much reply held for ordering",
			));
		}
		self.held_bytes += held_bytes;
		let kind = match kind {
			PartKind::Text => OpenKind::Text {
				unsent: OpenText::default(),
			},
			PartKind::Refusal => OpenKind::Refusal,
			PartKind::Reasoning => OpenKind::Reasoning,
			PartKind::ToolCall { call_id, name } => OpenKind::Call {
				call_id: call_id.to_string(),
				name: name.to_string(),
			},
		};
		self.parts.push_back(Part {
			id,
			kind,
			streaming: false,
			citations: Vec::new(),
			citation_bytes: 0,
			missed_delta: false,
			value: None,
			held_bytes,
		});
		Ok(())
	}

	fn delta(&mut self, id: PartId, text: JsonStr<'_>, sink: &mut dyn Sink) -> Result<(), Failure> {
		let Some(at) = self.parts.iter().position(|part| part.id == id) else {
			return Err(Failure::new(
				ErrorType::Api,
				"a part got text before it started",
			));
		};
		let part = &mut self.parts[at];
		part.missed_delta |= at > 0;
		if let OpenKind::Text { unsent } = &mut part.kind
			&& part.missed_delta
		{
			unsent.push_json(text);
			let now = unsent.len();
			let added = now - part.held_bytes;
			part.held_bytes = now;
			self.held_bytes += added;
			if self.held_bytes > self.max_held_bytes {
				return Err(Failure::new(
					ErrorType::Api,
					"too much reply held for ordering",
				));
			}
		}
		if matches!(part.kind, OpenKind::Call { .. }) || part.missed_delta {
			return Ok(());
		}
		let reasoning = matches!(part.kind, OpenKind::Reasoning);
		if !part.streaming {
			self.ensure_started(sink);
			sink.op(if reasoning {
				Op::ThinkingStart
			} else {
				Op::TextStart
			});
			if let Some(first) = self.parts.front_mut() {
				first.streaming = true;
			}
		}
		let raw = text.raw();
		sink.op(if reasoning {
			Op::Thinking { raw }
		} else {
			Op::Text { raw }
		});
		Ok(())
	}

	fn citation(
		&mut self,
		id: PartId,
		url: &str,
		title: Option<&str>,
		sink: &mut dyn Sink,
	) -> Result<(), Failure> {
		if let Some(first) = self
			.parts
			.front()
			.filter(|part| part.id == id && part.streaming)
		{
			debug_assert!(matches!(first.kind, OpenKind::Text { .. }));
			sink.op(Op::Citation { url, title });
		} else if let Some(part) = self.parts.iter_mut().find(|part| part.id == id) {
			let added = size_of::<HeldCitation>() + url.len() + title.map_or(0, str::len);
			if added > self.max_held_bytes.saturating_sub(self.held_bytes) {
				return Err(Failure::new(
					ErrorType::Api,
					"too much reply held for ordering",
				));
			}
			self.held_bytes += added;
			part.citation_bytes += added;
			part.citations.push(HeldCitation {
				url: url.to_string(),
				title: title.map(str::to_string),
			});
		}
		Ok(())
	}

	fn part_end(&mut self, id: PartId, value: Final<'_>, sink: &mut dyn Sink) -> Result<(), Failure> {
		let Some(part) = self.parts.iter_mut().find(|part| part.id == id) else {
			return Err(Failure::new(
				ErrorType::Api,
				"a part ended that never started",
			));
		};
		let value = complete(&mut part.kind, value, part.streaming)
			.ok_or_else(|| Failure::new(ErrorType::Api, "a part ended as a different kind"))?;
		let held_bytes = value.held_bytes();
		debug_assert!(held_bytes >= part.held_bytes);
		self.held_bytes += held_bytes - part.held_bytes;
		if self.held_bytes > self.max_held_bytes {
			return Err(Failure::new(
				ErrorType::Api,
				"too much reply held for ordering",
			));
		}
		part.value = Some(value);
		part.held_bytes = held_bytes;
		while let Some(completed) = self.parts.front_mut().and_then(Part::take_completed) {
			self.parts.pop_front();
			self.send(completed, sink);
		}
		Ok(())
	}

	fn send(&mut self, part: Completed, sink: &mut dyn Sink) {
		self.held_bytes -= part.held_bytes;
		match part.value {
			Value::Text { unsent, refusal } => {
				self.saw_refusal |= refusal;
				if let Some(raw) = unsent {
					if raw == EMPTY_TEXT {
						return;
					}
					self.ensure_started(sink);
					sink.op(Op::TextStart);
					sink.op(Op::Text { raw: &raw });
				}
				for citation in &part.citations {
					sink.op(Op::Citation {
						url: &citation.url,
						title: citation.title.as_deref(),
					});
				}
				sink.op(Op::BlockEnd);
			},
			Value::Call {
				call_id,
				name,
				arguments,
				complete,
			} => {
				if !complete {
					self.dropped_call = true;
					return;
				}
				self.saw_call = true;
				self.ensure_started(sink);
				sink.op(Op::ToolStart {
					id: &call_id,
					name: &name,
				});
				if arguments != EMPTY_TEXT {
					sink.op(Op::Arguments { raw: &arguments });
				}
				sink.op(Op::BlockEnd);
			},
			Value::Reasoning { unsent, item } => {
				if let Some(text) = unsent {
					self.ensure_started(sink);
					sink.op(Op::ThinkingStart);
					if text != EMPTY_TEXT {
						sink.op(Op::Thinking { raw: &text });
					}
				}
				let value = carrier::encode(
					self.scope,
					carrier::CarrierPurpose::MessagesSignature,
					item.as_bytes(),
				);
				sink.op(Op::Signature { value: &value });
				sink.op(Op::BlockEnd);
			},
		}
	}

	fn ensure_started(&mut self, sink: &mut dyn Sink) {
		if !std::mem::replace(&mut self.started, true) {
			let (id, model) = self
				.response
				.as_ref()
				.map_or(("", ""), |response| (&response.id, &response.model));
			sink.op(Op::Start { id, model });
		}
	}

	fn finish(&mut self, outcome: Outcome<'_>, sink: &mut dyn Sink) -> Result<(), Failure> {
		if !self.parts.is_empty() {
			return Err(Failure::new(
				ErrorType::Api,
				"the reply ended with a part unfinished",
			));
		}
		let stop_reason = match outcome {
			Outcome::Completed if self.dropped_call => {
				return Err(Failure::new(
					ErrorType::Api,
					"a completed reply held an unfinished tool call",
				));
			},
			Outcome::Completed if self.saw_call => "tool_use",
			Outcome::Completed if self.saw_refusal => "refusal",
			Outcome::Completed if self.stop_sequence.is_some() => "stop_sequence",
			Outcome::Completed => "end_turn",
			Outcome::Incomplete { reason, .. } => incomplete_stop_reason(reason)?,
		};
		self.ensure_started(sink);
		self.ended = true;
		sink.op(Op::End {
			stop_reason,
			stop_sequence: self
				.stop_sequence
				.as_deref()
				.filter(|_| stop_reason == "stop_sequence"),
			usage: self.usage,
			service_tier: self.service_tier,
		});
		Ok(())
	}
}

fn from_provider(error: ProviderError<'_>) -> Failure {
	let mut failure = classify(error.kind, None);
	if let Some(message) = error.client_message {
		failure.message = message.to_string();
	}
	failure
}

fn incomplete_stop_reason(reason: StopReason) -> Result<&'static str, Failure> {
	match reason {
		StopReason::MaxTokens => Ok("max_tokens"),
		StopReason::ContextWindow => Ok("model_context_window_exceeded"),
		StopReason::ContentFilter => Ok("refusal"),
		StopReason::Other => Err(Failure::new(
			ErrorType::Api,
			"the provider stopped early with no usable reason",
		)),
	}
}

const EMPTY_TEXT: &str = "\"\"";

impl Value {
	/// Streamed text isn't kept, so it doesn't count: `max_block_bytes` and
	/// `max_output_bytes` bound it.
	fn held_bytes(&self) -> usize {
		match self {
			Self::Text { unsent, .. } => unsent.as_ref().map_or(0, String::len),
			Self::Call {
				call_id,
				name,
				arguments,
				..
			} => call_id.len() + name.len() + arguments.len(),
			Self::Reasoning { unsent, item } => unsent.as_ref().map_or(0, String::len) + item.len(),
		}
	}
}

/// `None` when the final value isn't of the kind the part started as.
fn complete(kind: &mut OpenKind, value: Final<'_>, streamed: bool) -> Option<Value> {
	let unsent = |text: Option<JsonStr<'_>>| {
		(!streamed).then(|| text.map_or(EMPTY_TEXT, |text| text.raw()).to_string())
	};
	match (kind, value) {
		(OpenKind::Text { unsent: text }, Final::Text) => Some(Value::Text {
			unsent: (!streamed).then(|| std::mem::take(text).finish()),
			refusal: false,
		}),
		(OpenKind::Refusal, Final::Refusal(text)) => Some(Value::Text {
			unsent: unsent(Some(text)),
			refusal: true,
		}),
		(OpenKind::Call { call_id, name }, Final::ToolCall(arguments)) => Some(Value::Call {
			call_id: std::mem::take(call_id),
			name: std::mem::take(name),
			arguments: arguments.raw().to_string(),
			complete: arguments.is_complete(),
		}),
		(OpenKind::Reasoning, Final::Reasoning { text, item, .. }) => Some(Value::Reasoning {
			unsent: unsent(text),
			item: item.get().to_string(),
		}),
		_ => None,
	}
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
