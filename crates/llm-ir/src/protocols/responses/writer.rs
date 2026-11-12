//! One state builds both a stream and a buffered reply.
//!
//! Nothing goes out until the first part, so an early failure can still be an
//! HTTP error. Parts can overlap: only the first open part streams, and later
//! ones wait for their final value. Tool calls go out only at their final value;
//! truncated calls are omitted so clients cannot dispatch them.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};

use super::wire::{
	ErrorBody, ErrorDetail, Execution, IncompleteDetails, InputTokensDetails, Item, ItemStatus,
	OutputItem, OutputText, OutputTokensDetails, Response, ResponseError, ResponseUsage, Role,
	Status, StreamEvent, SummaryText, Text, UrlCitation,
};
use crate::carrier::{self, StateScope};
use crate::ir::events::{
	ErrorKind, Event, Final, JsonStr, OpenText, Outcome, PartId, PartKind, StopReason, Usage,
};
use crate::json::is_strict_object;
use crate::protocols::{CallKind, HttpReply, ToolRoute, is_field_name};
use crate::sse::{close_event, open_event};

/// Room for a response's fields around its output, so a buffered reply doesn't
/// grow its buffer, and double it, at the last moment.
const RESPONSE_OVERHEAD: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
	/// Call `take_stream`.
	HasOutput,
	/// Feed more, or call `fail` at the end of input.
	NeedsInput,
	/// Check `early_failure`.
	Finished,
}

pub struct ResponseStream<'a> {
	state: ResponseState<'a, EventStream>,
}

impl<'a> ResponseStream<'a> {
	/// `max_output_bytes` bounds what's kept for the terminal event.
	#[must_use]
	pub fn new(
		scope: StateScope<'a>,
		routes: &'a [ToolRoute],
		created_at: u64,
		max_output_bytes: usize,
	) -> Self {
		Self {
			state: ResponseState::new(
				scope,
				routes,
				created_at,
				max_output_bytes,
				EventStream::default(),
			),
		}
	}

	/// Only for a client that asked: thoughts aren't meant to be shown otherwise.
	#[must_use]
	pub const fn show_reasoning(mut self, show: bool) -> Self {
		self.state.show_reasoning = show;
		self
	}

	pub fn event(&mut self, event: &Event<'_>) {
		self.state.event(event);
	}

	/// Later events are ignored.
	pub fn fail(&mut self, status: u16) {
		self.state.fail("server_error", status);
	}

	/// `HasOutput` comes first: an ended reply isn't finished until it's drained.
	#[must_use]
	pub fn progress(&self) -> Progress {
		if self.state.has_output() {
			Progress::HasOutput
		} else if self.state.ended {
			Progress::Finished
		} else {
			Progress::NeedsInput
		}
	}

	/// A terminal event comes one output item at a time, and waiting parts one
	/// part at a time. Each is freed once taken.
	pub fn take_stream(&mut self) -> Vec<u8> {
		self.state.take_stream()
	}

	/// The HTTP reply for a failure before any output. Output starts when the
	/// first bytes are queued, not when they're taken. After that this is always
	/// `None`, and a failure travels in the stream. When it's `Some`,
	/// `take_stream` has nothing, and the caller sends this instead.
	#[must_use]
	pub fn early_failure(&self) -> Option<HttpReply> {
		match self.state.reply.failure {
			Some((code, status)) if !self.state.started => Some(HttpReply {
				status,
				body: self.state.reply.failure_body(code),
			}),
			_ => None,
		}
	}
}

pub struct BufferedResponse<'a> {
	state: ResponseState<'a, NoEvents>,
}

impl<'a> BufferedResponse<'a> {
	/// `max_output_bytes` bounds the reply's output.
	#[must_use]
	pub fn new(
		scope: StateScope<'a>,
		routes: &'a [ToolRoute],
		created_at: u64,
		max_output_bytes: usize,
	) -> Self {
		Self {
			state: ResponseState::new(scope, routes, created_at, max_output_bytes, NoEvents),
		}
	}

	/// Only for a client that asked: thoughts aren't meant to be shown otherwise.
	#[must_use]
	pub const fn show_reasoning(mut self, show: bool) -> Self {
		self.state.show_reasoning = show;
		self
	}

	pub fn event(&mut self, event: &Event<'_>) {
		self.state.event(event);
		self.write_waiting();
	}

	/// Later events are ignored.
	pub fn fail(&mut self, status: u16) {
		self.state.fail("server_error", status);
		self.write_waiting();
	}

	/// A reply that never ended is a 500, so a caller whose input stopped early
	/// should say why with `fail`.
	#[must_use]
	pub fn into_reply(self) -> HttpReply {
		let state = self.state;
		match state.reply.failure {
			Some((code, status)) => HttpReply {
				status,
				body: state.reply.failure_body(code),
			},
			None if state.ended => HttpReply {
				status: 200,
				body: state.response_body(),
			},
			None => HttpReply {
				status: 500,
				body: error_body("server_error"),
			},
		}
	}

	fn write_waiting(&mut self) {
		while self.state.write_next() {}
	}
}

struct ResponseState<'a, F> {
	scope: StateScope<'a>,
	routes: &'a [ToolRoute],
	max_output_bytes: usize,
	show_reasoning: bool,
	reply: ResponseEnvelope,
	events: F,
	started: bool,
	ended: bool,
	/// Parts not fully written, in start order. Only the first can stream.
	parts: VecDeque<OpenPart>,
	output: Vec<Box<RawValue>>,
	output_bytes: usize,
	held_bytes: usize,
	open_text_bytes: usize,
	incomplete_call: bool,
	/// Waits for the parts ahead of it to be written.
	ending: Option<Ending>,
}

pub(super) struct ResponseEnvelope {
	id: String,
	model: String,
	created_at: u64,
	usage: Usage,
	incomplete: Option<&'static str>,
	failure: Option<(&'static str, u16)>,
	/// What the client is told of the failure, in place of its code's fixed text.
	client_message: Option<String>,
}

struct OpenPart {
	id: PartId,
	kind: Kind,
	identifier_bytes: usize,
	citations: Vec<UrlCitation>,
	citation_bytes: usize,
	/// The final value of a part still waiting for an earlier one.
	held: Option<Done<'static>>,
	/// A delta went out, so the deltas so far are all the text so far.
	sent_delta: bool,
	/// A delta came while this part waited. Its text goes out whole at the end.
	missed_delta: bool,
	/// Answer text from its deltas, which a final value doesn't include.
	text: Option<OpenText>,
}

enum Kind {
	Text,
	Streaming {
		item_id: String,
		output_index: usize,
		reasoning: bool,
	},
	Reasoning,
	Call {
		call_id: String,
		name: String,
	},
}

#[derive(Clone, Copy)]
enum Ending {
	Completed,
	Incomplete(StopReason),
}

/// A part's final value, borrowed unless it has to wait. Text is a JSON string.
enum Done<'a> {
	Text(Cow<'a, RawValue>),
	Call {
		arguments: Cow<'a, RawValue>,
		complete: bool,
	},
	Reasoning {
		item: Cow<'a, RawValue>,
		text: Option<Cow<'a, RawValue>>,
	},
}

impl Done<'_> {
	fn size(&self) -> usize {
		match self {
			Self::Text(value)
			| Self::Call {
				arguments: value, ..
			} => value.get().len(),
			Self::Reasoning { item, text } => {
				item.get().len() + text.as_ref().map_or(0, |text| text.get().len())
			},
		}
	}

	fn into_owned(self) -> Done<'static> {
		match self {
			Self::Text(text) => Done::Text(Cow::Owned(text.into_owned())),
			Self::Call {
				arguments,
				complete,
			} => Done::Call {
				arguments: Cow::Owned(arguments.into_owned()),
				complete,
			},
			Self::Reasoning { item, text } => Done::Reasoning {
				item: Cow::Owned(item.into_owned()),
				text: text.map(|text| Cow::Owned(text.into_owned())),
			},
		}
	}
}

impl<'a> From<Final<'a>> for Done<'a> {
	fn from(value: Final<'a>) -> Self {
		match value {
			Final::Text => unreachable!("answer text comes from its deltas"),
			Final::Refusal(text) => Self::Text(Cow::Borrowed(text.raw_value())),
			Final::ToolCall(arguments) => Self::Call {
				arguments: Cow::Borrowed(arguments.raw_value()),
				complete: arguments.is_complete(),
			},
			Final::Reasoning { item, text, .. } => Self::Reasoning {
				item: Cow::Borrowed(item),
				text: text.map(|text| Cow::Borrowed(text.raw_value())),
			},
		}
	}
}

#[derive(Deserialize)]
struct CustomInput {
	input: String,
}

impl ResponseState<'_, EventStream> {
	fn has_output(&self) -> bool {
		!self.events.bytes.is_empty() || self.events.pending.is_some() || self.next_is_ready()
	}

	fn take_stream(&mut self) -> Vec<u8> {
		while self.events.bytes.is_empty() && self.events.pending.is_none() && self.write_next() {}
		let mut out = std::mem::take(&mut self.events.bytes);
		if let Some(pending) = &mut self.events.pending {
			if let Some(item) = pending.items.next() {
				if std::mem::replace(&mut pending.sent_item, true) {
					out.push(b',');
				}
				out.extend_from_slice(item.get().as_bytes());
			}
			if pending.items.len() == 0 {
				out.extend_from_slice(&pending.tail);
				close_event(&mut out);
				self.events.pending = None;
			}
		}
		out
	}
}

impl<'a, F: ReplyEvents> ResponseState<'a, F> {
	fn new(
		scope: StateScope<'a>,
		routes: &'a [ToolRoute],
		created_at: u64,
		max_output_bytes: usize,
		events: F,
	) -> Self {
		Self {
			scope,
			routes,
			max_output_bytes,
			show_reasoning: false,
			reply: ResponseEnvelope {
				id: String::new(),
				model: String::new(),
				created_at,
				usage: Usage::default(),
				incomplete: None,
				failure: None,
				client_message: None,
			},
			events,
			started: false,
			ended: false,
			parts: VecDeque::new(),
			output: Vec::new(),
			output_bytes: 0,
			held_bytes: 0,
			open_text_bytes: 0,
			incomplete_call: false,
			ending: None,
		}
	}

	fn next_is_ready(&self) -> bool {
		match self.parts.front() {
			Some(part) => part.held.is_some(),
			None => self.ending.is_some(),
		}
	}

	/// False when neither a waiting part nor the ending is ready.
	fn write_next(&mut self) -> bool {
		if !self.next_is_ready() {
			return false;
		}
		self.open_first();
		match self.parts.pop_front() {
			Some(mut part) => {
				self.held_bytes -= part.identifier_bytes + part.citation_bytes;
				if let Some(done) = part.held.take() {
					self.held_bytes -= done.size();
					self.write_part(part, done);
				}
				self.open_next();
			},
			None => {
				if let Some(ending) = self.ending.take() {
					self.apply_ending(ending);
				}
			},
		}
		true
	}

	fn event(&mut self, event: &Event<'_>) {
		if self.ended || self.ending.is_some() {
			return;
		}
		match *event {
			Event::ResponseStart { id, model } => {
				self.reply.id = format!("resp_{id}");
				self.reply.model = model.to_string();
			},
			Event::PartStart { part, kind } => self.part_start(part, kind),
			Event::Delta { part, text } => self.delta(part, text),
			Event::Citation { part, url, title } => {
				if let Some(open) = self.parts.iter_mut().find(|open| open.id == part) {
					let added = size_of::<UrlCitation>() + url.len() + title.map_or(0, str::len);
					self.held_bytes += added;
					open.citation_bytes += added;
					if self.output_bytes + self.held_bytes + self.open_text_bytes > self.max_output_bytes {
						self.fail("server_error", 500);
						return;
					}
					open.citations.push(UrlCitation {
						url: url.to_string(),
						title: title.map(str::to_string),
					});
				}
			},
			Event::PartEnd { part, value } => self.part_end(part, value),
			Event::Usage(usage) => self.reply.usage = usage,
			Event::Finish(outcome) => self.finish(outcome),
			Event::Error(error) => {
				let (code, status) = error_code(error.kind);
				self.fail_saying(code, status, error.client_message);
			},
			Event::MessageEnd { .. } | Event::ServiceTier(_) | Event::StopSequence(_) => {},
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
		self.reply.client_message = message.map(str::to_string);
		self.ended = true;
		self.parts.clear();
		self.held_bytes = 0;
		self.open_text_bytes = 0;
		self.ending = None;
		self.reply.failure = Some((code, status));
		if self.started {
			self.emit_response(Status::Failed);
		}
	}

	const fn status(&self) -> Status {
		if self.reply.incomplete.is_some() {
			Status::Incomplete
		} else {
			Status::Completed
		}
	}

	fn part_start(&mut self, id: PartId, kind: PartKind<'_>) {
		let identifier_bytes = match kind {
			PartKind::ToolCall { call_id, name } => call_id.len() + name.len(),
			_ => 0,
		};
		self.held_bytes += identifier_bytes;
		if self.over_limit() {
			self.fail("server_error", 500);
			return;
		}
		let text = matches!(kind, PartKind::Text).then(OpenText::default);
		self.open_text_bytes += text.as_ref().map_or(0, OpenText::len);
		let kind = match kind {
			PartKind::Text | PartKind::Refusal => Kind::Text,
			PartKind::Reasoning => Kind::Reasoning,
			PartKind::ToolCall { call_id, name } => Kind::Call {
				call_id: call_id.to_string(),
				name: name.to_string(),
			},
		};
		self.parts.push_back(OpenPart {
			id,
			kind,
			identifier_bytes,
			citations: Vec::new(),
			citation_bytes: 0,
			held: None,
			sent_delta: false,
			missed_delta: false,
			text,
		});
		if self.over_limit() {
			self.fail("server_error", 500);
		} else {
			self.open_next();
		}
	}

	fn delta(&mut self, id: PartId, text: JsonStr<'_>) {
		let Some(at) = self.parts.iter().position(|part| part.id == id) else {
			self.fail("server_error", 500);
			return;
		};
		let added = if let Some(so_far) = &mut self.parts[at].text {
			let before = so_far.len();
			so_far.push_json(text);
			so_far.len() - before
		} else {
			0
		};
		self.open_text_bytes += added;
		if self.over_limit() {
			self.fail("server_error", 500);
			return;
		}
		let part = &mut self.parts[at];
		let Kind::Streaming {
			item_id,
			output_index,
			reasoning,
		} = &part.kind
		else {
			part.missed_delta = true;
			return;
		};
		if part.missed_delta {
			return;
		}
		let first = !std::mem::replace(&mut part.sent_delta, true);
		if !reasoning {
			self
				.events
				.text_delta(item_id, *output_index, text.raw_value());
			return;
		}
		if first {
			self.events.summary_opened(item_id, *output_index);
		}
		self
			.events
			.summary_delta(item_id, *output_index, text.raw_value());
	}

	fn open_next(&mut self) {
		if self.parts.front().is_some_and(|part| part.held.is_none()) {
			self.open_first();
		}
	}

	fn open_first(&mut self) {
		let reasoning = match self.parts.front() {
			Some(OpenPart {
				kind: Kind::Text, ..
			}) => false,
			Some(OpenPart {
				kind: Kind::Reasoning,
				..
			}) if self.show_reasoning => true,
			_ => return,
		};
		self.ensure_started();
		let output_index = self.output.len();
		let item_id = if reasoning {
			let item_id = format!("rs_{output_index}");
			let item = OutputItem::Reasoning {
				id: &item_id,
				summary: &[],
				encrypted_content: None,
			};
			self.events.item_added(output_index, Item::Open(item));
			item_id
		} else {
			let item_id = format!("msg_{output_index}");
			self.events.text_opened(&item_id, output_index);
			item_id
		};
		if let Some(first) = self.parts.front_mut() {
			first.kind = Kind::Streaming {
				item_id,
				output_index,
				reasoning,
			};
		}
	}

	fn ensure_started(&mut self) {
		if !std::mem::replace(&mut self.started, true) {
			self.emit_response(Status::InProgress);
		}
	}

	fn part_end(&mut self, id: PartId, value: Final<'_>) {
		let Some(at) = self.parts.iter().position(|part| part.id == id) else {
			self.fail("server_error", 500);
			return;
		};
		let done = match value {
			Final::Text => match self.parts.get_mut(at).and_then(|part| part.text.take()) {
				Some(text) => {
					self.open_text_bytes -= text.len();
					match RawValue::from_string(text.finish()) {
						Ok(text) => Done::Text(Cow::Owned(text)),
						Err(error) => unreachable!("deltas join into one JSON string: {error}"),
					}
				},
				None => {
					self.fail("server_error", 500);
					return;
				},
			},
			value => Done::from(value),
		};
		if at > 0 {
			self.hold(at, done);
			return;
		}
		if let Some(part) = self.parts.pop_front() {
			self.held_bytes -= part.identifier_bytes + part.citation_bytes;
			self.write_part(part, done);
		}
		self.open_next();
	}

	fn hold(&mut self, at: usize, done: Done<'_>) {
		self.held_bytes += done.size();
		if self.over_limit() {
			self.fail("server_error", 500);
		} else if let Some(part) = self.parts.get_mut(at) {
			part.held = Some(done.into_owned());
		}
	}

	fn write_part(&mut self, part: OpenPart, done: Done<'_>) {
		match (part.kind, done) {
			(
				Kind::Streaming {
					item_id,
					output_index,
					reasoning: false,
				},
				Done::Text(text),
			) => {
				if !part.sent_delta && text.get() != r#""""# {
					self.events.text_delta(&item_id, output_index, &text);
				}
				let content = OutputText {
					text: Text::Escaped(&text),
					annotations: &part.citations,
				};
				let item = to_json(&OutputItem::Message {
					id: &item_id,
					role: Role::Assistant,
					status: ItemStatus::Completed,
					content: std::slice::from_ref(&content),
				});
				self
					.events
					.text_done(&item_id, output_index, &text, &content);
				// The item owns the text now. Free its separate JSON string before
				// item_done copies the item into the outgoing event.
				drop(text);
				self.finish_item(item);
			},
			(
				Kind::Streaming {
					item_id,
					output_index,
					reasoning: true,
				},
				Done::Reasoning { item, text },
			) => {
				let text = text.filter(|text| part.sent_delta || text.get() != r#""""#);
				if let Some(text) = &text {
					if !part.sent_delta {
						self.events.summary_opened(&item_id, output_index);
						self.events.summary_delta(&item_id, output_index, text);
					}
					self.events.summary_done(&item_id, output_index, text);
				}
				let out = self.reasoning_item(&item, text.as_deref());
				self.finish_item(out);
			},
			(Kind::Reasoning, Done::Reasoning { item, text }) => {
				let shown = text.filter(|text| self.show_reasoning && text.get() != r#""""#);
				let out = self.reasoning_item(&item, shown.as_deref());
				self.ensure_started();
				self.announce_and_finish(out);
			},
			(
				Kind::Call { call_id, name },
				Done::Call {
					arguments,
					complete,
				},
			) => {
				if !complete {
					self.incomplete_call = true;
					// Clients can dispatch output_item.done even when its item says incomplete.
					return;
				}
				match self.call_item(&call_id, &name, &arguments) {
					Some(item) => {
						self.ensure_started();
						self.announce_and_finish(item);
					},
					None => self.fail("server_error", 500),
				}
			},
			_ => self.fail("server_error", 500),
		}
	}

	/// Restores what the request flattened: the namespace, custom tools and tool search.
	fn call_item(&self, call_id: &str, name: &str, arguments: &RawValue) -> Option<Box<RawValue>> {
		let id = |prefix: &str| format!("{prefix}_{}", self.output.len());
		let route = self.routes.iter().find(|route| route.flat == name);
		let name = route.map_or(name, |route| &route.name);
		let text: String = serde_json::from_str(arguments.get()).ok()?;
		// A call that takes no arguments can stream none.
		let no_arguments;
		let (text, arguments) = if text.is_empty() {
			no_arguments = to_raw_value("{}").ok()?;
			("{}", &*no_arguments)
		} else {
			(text.as_str(), arguments)
		};
		let item = match route.map_or(CallKind::Function, |route| route.kind) {
			CallKind::Custom => {
				let CustomInput { input } = serde_json::from_str(text).ok()?;
				to_json(&OutputItem::CustomToolCall {
					id: &id("ctc"),
					call_id,
					name,
					input: &input,
					status: ItemStatus::Completed,
				})
			},
			CallKind::ToolSearch => {
				let object: &RawValue = serde_json::from_str::<&RawValue>(text).ok()?;
				if !is_strict_object(object) {
					return None;
				}
				to_json(&OutputItem::ToolSearchCall {
					id: &id("tsc"),
					call_id,
					execution: Execution::Client,
					arguments: object,
					status: ItemStatus::Completed,
				})
			},
			CallKind::Function => {
				if !is_strict_object(serde_json::from_str::<&RawValue>(text).ok()?) {
					return None;
				}
				to_json(&OutputItem::FunctionCall {
					id: &id("fc"),
					call_id,
					name,
					namespace: route.and_then(|route| route.namespace.as_deref()),
					arguments,
					status: ItemStatus::Completed,
				})
			},
		};
		Some(item)
	}

	fn reasoning_item(&self, item: &RawValue, summary: Option<&RawValue>) -> Box<RawValue> {
		let carrier = carrier::encode(
			self.scope,
			carrier::CarrierPurpose::ResponsesEncryptedContent,
			item.get().as_bytes(),
		);
		let summary = summary.map(|text| SummaryText {
			text: Text::Escaped(text),
		});
		to_json(&OutputItem::Reasoning {
			id: &format!("rs_{}", self.output.len()),
			summary: summary.as_slice(),
			encrypted_content: Some(&carrier),
		})
	}

	fn announce_and_finish(&mut self, item: Box<RawValue>) {
		if self.admit(&item) {
			self
				.events
				.item_added(self.output.len(), Item::Written(&item));
			self.push_item(item);
		}
	}

	fn finish_item(&mut self, item: Box<RawValue>) {
		if self.admit(&item) {
			self.push_item(item);
		}
	}

	fn admit(&mut self, item: &RawValue) -> bool {
		self.output_bytes += item.get().len();
		if self.over_limit() {
			self.fail("server_error", 500);
			return false;
		}
		true
	}

	const fn over_limit(&self) -> bool {
		self.output_bytes + self.held_bytes + self.open_text_bytes > self.max_output_bytes
	}

	fn push_item(&mut self, item: Box<RawValue>) {
		self.events.item_done(self.output.len(), &item);
		self.output.push(item);
	}

	fn finish(&mut self, outcome: Outcome<'_>) {
		if self.parts.iter().any(|part| part.held.is_none()) {
			self.fail("server_error", 500);
			return;
		}
		let ending = match outcome {
			Outcome::Completed => Ending::Completed,
			Outcome::Incomplete { reason, .. } => Ending::Incomplete(reason),
		};
		if self.parts.is_empty() {
			self.apply_ending(ending);
		} else {
			self.ending = Some(ending);
		}
	}

	fn apply_ending(&mut self, ending: Ending) {
		match ending {
			Ending::Completed if self.incomplete_call => self.fail("server_error", 500),
			Ending::Completed => self.end(Status::Completed),
			Ending::Incomplete(reason) => match reason {
				StopReason::MaxTokens => self.end_incomplete("max_output_tokens"),
				StopReason::ContentFilter => self.end_incomplete("content_filter"),
				StopReason::ContextWindow => self.fail("context_length_exceeded", 400),
				StopReason::Other => self.fail("server_error", 500),
			},
		}
	}

	fn end_incomplete(&mut self, reason: &'static str) {
		self.reply.incomplete = Some(reason);
		self.end(Status::Incomplete);
	}

	fn end(&mut self, status: Status) {
		self.ensure_started();
		self.ended = true;
		self.emit_response(status);
	}

	fn emit_response(&mut self, status: Status) {
		self.events.response(status, &self.reply, &mut self.output);
	}

	fn response_body(&self) -> Vec<u8> {
		let mut out = Vec::with_capacity(RESPONSE_OVERHEAD + self.output_bytes + self.output.len());
		let response = self.reply.response(self.status(), &self.output);
		if let Err(error) = serde_json::to_writer(&mut out, &response) {
			unreachable!("a response holds only strings, numbers and checked JSON: {error}");
		}
		out
	}
}

impl ResponseEnvelope {
	pub(super) fn response<'a>(
		&'a self,
		status: Status,
		output: &'a [Box<RawValue>],
	) -> Response<'a> {
		Response {
			id: &self.id,
			object: "response",
			created_at: self.created_at,
			model: &self.model,
			status,
			output,
			usage: (status != Status::InProgress).then(|| response_usage(self.usage)),
			error: self.failure.map(|(code, _)| ResponseError {
				code,
				message: self.failure_message(code),
			}),
			incomplete_details: match self.failure {
				Some(_) => None,
				None => self.incomplete.map(|reason| IncompleteDetails { reason }),
			},
		}
	}
	fn failure_body(&self, code: &'static str) -> Vec<u8> {
		error_json(code, Cow::Borrowed(self.failure_message(code)))
	}

	fn failure_message(&self, code: &'static str) -> &str {
		self
			.client_message
			.as_deref()
			.unwrap_or_else(|| message(code))
	}
}

/// A provider's HTTP error as a client's. An error with no known meaning is still
/// a server failure, so a client retries it.
#[must_use]
pub fn error_reply(status: u16, kind: ErrorKind) -> HttpReply {
	let (code, mapped) = error_code(kind);
	let status = if kind == ErrorKind::Unknown {
		status.max(500)
	} else {
		mapped
	};
	HttpReply {
		status,
		body: error_body(code),
	}
}

/// `error_reply`, naming the field a provider refused. A field name is the client's
/// own vocabulary, where the provider's words aren't.
#[must_use]
pub fn error_reply_naming(status: u16, kind: ErrorKind, field: Option<&str>) -> HttpReply {
	let (code, _) = error_code(kind);
	match field.filter(|field| code == "invalid_prompt" && is_field_name(field)) {
		Some(field) => HttpReply {
			status: error_reply(status, kind).status,
			body: error_json(code, Cow::Owned(format!("The provider refused `{field}`."))),
		},
		None => error_reply(status, kind),
	}
}

/// Responses requires token counts, so unknown is written as 0 here. Accounting
/// reads the usage fold, where unknown stays unknown.
fn response_usage(usage: Usage) -> ResponseUsage {
	let input_tokens = usage.input_tokens.unwrap_or(0);
	let output_tokens = usage.output_tokens.unwrap_or(0);
	ResponseUsage {
		input_tokens,
		input_tokens_details: InputTokensDetails {
			cached_tokens: usage.cached_tokens.unwrap_or(0),
			cache_write_tokens: usage.cache_write_tokens,
		},
		output_tokens,
		output_tokens_details: OutputTokensDetails {
			reasoning_tokens: usage.reasoning_tokens.unwrap_or(0),
		},
		total_tokens: usage
			.total_tokens
			.unwrap_or_else(|| input_tokens.saturating_add(output_tokens)),
	}
}

/// Only codes this client knows, with fixed text. The provider's words stay in logs.
pub(crate) const fn error_code(kind: ErrorKind) -> (&'static str, u16) {
	match kind {
		ErrorKind::RateLimit => ("rate_limit_exceeded", 429),
		ErrorKind::Overloaded => ("server_is_overloaded", 503),
		ErrorKind::InvalidRequest => ("invalid_prompt", 400),
		ErrorKind::Authentication => ("invalid_api_key", 401),
		ErrorKind::Permission => ("permission_denied", 403),
		ErrorKind::Billing => ("insufficient_quota", 429),
		ErrorKind::ContextTooLong | ErrorKind::RequestTooLarge => ("context_length_exceeded", 400),
		ErrorKind::NotFound => ("model_not_found", 404),
		ErrorKind::Unknown => ("server_error", 500),
	}
}

pub(crate) fn message(code: &str) -> &'static str {
	match code {
		"rate_limit_exceeded" => "The provider's rate limit was reached.",
		"server_is_overloaded" => "The provider is overloaded.",
		"invalid_prompt" => "The provider refused the request.",
		"invalid_api_key" => "The gateway's provider credentials were refused.",
		"permission_denied" => "The provider refused access.",
		"insufficient_quota" => "The provider's quota is used up.",
		"context_length_exceeded" => "The conversation is too long for this model.",
		"model_not_found" => "The provider doesn't have this model.",
		_ => "The provider failed.",
	}
}

fn error_body(code: &str) -> Vec<u8> {
	error_json(code, Cow::Borrowed(message(code)))
}

fn error_json(code: &str, message: Cow<'_, str>) -> Vec<u8> {
	let body = ErrorBody {
		error: ErrorDetail {
			kind: code,
			code,
			message,
		},
	};
	match serde_json::to_vec(&body) {
		Ok(body) => body,
		Err(error) => unreachable!("an error holds only strings: {error}"),
	}
}

/// Output items hold only strings, numbers and checked JSON, so they always serialize.
fn to_json(item: &impl Serialize) -> Box<RawValue> {
	match to_raw_value(item) {
		Ok(item) => item,
		Err(error) => unreachable!("an output item always serializes: {error}"),
	}
}

pub(super) trait ReplyEvents {
	fn text_opened(&mut self, _item_id: &str, _output_index: usize) {}
	fn text_delta(&mut self, _item_id: &str, _output_index: usize, _text: &RawValue) {}
	fn text_done(
		&mut self,
		_item_id: &str,
		_output_index: usize,
		_text: &RawValue,
		_part: &OutputText<'_>,
	) {
	}
	fn summary_opened(&mut self, _item_id: &str, _output_index: usize) {}
	fn summary_delta(&mut self, _item_id: &str, _output_index: usize, _text: &RawValue) {}
	fn summary_done(&mut self, _item_id: &str, _output_index: usize, _text: &RawValue) {}
	fn item_added(&mut self, _output_index: usize, _item: Item<'_>) {}
	fn item_done(&mut self, _output_index: usize, _item: &RawValue) {}
	/// Takes `output` when it sends the response's items.
	fn response(
		&mut self,
		_status: Status,
		_reply: &ResponseEnvelope,
		_output: &mut Vec<Box<RawValue>>,
	) {
	}
}

pub(super) struct NoEvents;

impl ReplyEvents for NoEvents {}

#[derive(Default)]
pub(super) struct EventStream {
	pub(super) bytes: Vec<u8>,
	sequence: u64,
	pub(super) pending: Option<PendingTerminal>,
}

/// A terminal event with its output items still to send. `tail` ends its JSON.
pub(super) struct PendingTerminal {
	pub(super) items: std::vec::IntoIter<Box<RawValue>>,
	pub(super) sent_item: bool,
	pub(super) tail: Vec<u8>,
}

impl ReplyEvents for EventStream {
	fn text_opened(&mut self, item_id: &str, output_index: usize) {
		let item = OutputItem::Message {
			id: item_id,
			role: Role::Assistant,
			status: ItemStatus::InProgress,
			content: &[],
		};
		let sequence_number = self.next_sequence();
		self.emit(&StreamEvent::ItemAdded {
			output_index,
			item: Item::Open(item),
			sequence_number,
		});
		let part = OutputText {
			text: Text::Empty,
			annotations: &[],
		};
		let sequence_number = self.next_sequence();
		self.emit(&StreamEvent::ContentPartAdded {
			item_id,
			output_index,
			content_index: 0,
			part: &part,
			sequence_number,
		});
	}

	fn text_delta(&mut self, item_id: &str, output_index: usize, text: &RawValue) {
		self.emit_delta(DeltaKind::Text, item_id, output_index, text);
	}

	fn text_done(
		&mut self,
		item_id: &str,
		output_index: usize,
		text: &RawValue,
		part: &OutputText<'_>,
	) {
		let sequence_number = self.next_sequence();
		self.emit(&StreamEvent::TextDone {
			item_id,
			output_index,
			content_index: 0,
			text,
			sequence_number,
		});
		let sequence_number = self.next_sequence();
		self.emit(&StreamEvent::ContentPartDone {
			item_id,
			output_index,
			content_index: 0,
			part,
			sequence_number,
		});
	}

	fn summary_opened(&mut self, item_id: &str, output_index: usize) {
		let sequence_number = self.next_sequence();
		self.emit(&StreamEvent::SummaryPartAdded {
			item_id,
			output_index,
			summary_index: 0,
			part: SummaryText { text: Text::Empty },
			sequence_number,
		});
	}

	fn summary_delta(&mut self, item_id: &str, output_index: usize, text: &RawValue) {
		self.emit_delta(DeltaKind::Summary, item_id, output_index, text);
	}

	fn summary_done(&mut self, item_id: &str, output_index: usize, text: &RawValue) {
		let sequence_number = self.next_sequence();
		self.emit(&StreamEvent::SummaryDone {
			item_id,
			output_index,
			summary_index: 0,
			text,
			sequence_number,
		});
		let sequence_number = self.next_sequence();
		self.emit(&StreamEvent::SummaryPartDone {
			item_id,
			output_index,
			summary_index: 0,
			part: SummaryText {
				text: Text::Escaped(text),
			},
			sequence_number,
		});
	}

	fn item_added(&mut self, output_index: usize, item: Item<'_>) {
		let sequence_number = self.next_sequence();
		self.emit(&StreamEvent::ItemAdded {
			output_index,
			item,
			sequence_number,
		});
	}

	fn item_done(&mut self, output_index: usize, item: &RawValue) {
		let sequence_number = self.next_sequence();
		self.emit(&StreamEvent::ItemDone {
			output_index,
			item: Item::Written(item),
			sequence_number,
		});
	}

	/// A response with output goes out one item at a time, from `take_stream`, so
	/// its event is written without them and split where they go.
	fn response(
		&mut self,
		status: Status,
		reply: &ResponseEnvelope,
		output: &mut Vec<Box<RawValue>>,
	) {
		let sequence_number = self.next_sequence();
		let response = reply.response(status, &[]);
		let event = match status {
			Status::InProgress => StreamEvent::Created {
				response,
				sequence_number,
			},
			Status::Completed => StreamEvent::Completed {
				response,
				sequence_number,
			},
			Status::Incomplete => StreamEvent::Incomplete {
				response,
				sequence_number,
			},
			Status::Failed => StreamEvent::Failed {
				response,
				sequence_number,
			},
		};
		if output.is_empty() {
			self.emit(&event);
			return;
		}
		open_event(&mut self.bytes, event.name());
		let mut json = event_json(&event);
		let tail = json.split_off(output_start(&json));
		self.bytes.extend_from_slice(&json);
		self.pending = Some(PendingTerminal {
			items: std::mem::take(output).into_iter(),
			sent_item: false,
			tail,
		});
	}
}

impl EventStream {
	/// Writes straight into `bytes`, so a large item isn't copied on the way.
	fn emit(&mut self, event: &StreamEvent<'_>) {
		open_event(&mut self.bytes, event.name());
		if let Err(error) = serde_json::to_writer(&mut self.bytes, event) {
			unreachable!("events hold only strings, numbers and checked JSON: {error}");
		}
		close_event(&mut self.bytes);
	}

	/// Writes frequent deltas without the serializer. `item_id` is generated as
	/// `msg_N` or `rs_N`, and `text` is checked JSON, so neither needs escaping.
	fn emit_delta(&mut self, kind: DeltaKind, item_id: &str, output_index: usize, text: &RawValue) {
		let sequence_number = self.next_sequence();
		let (name, index) = match kind {
			DeltaKind::Text => ("response.output_text.delta", "content_index"),
			DeltaKind::Summary => ("response.reasoning_summary_text.delta", "summary_index"),
		};
		open_event(&mut self.bytes, name);
		let _ = write!(
			self.bytes,
			r#"{{"type":"{name}","item_id":"{item_id}","output_index":{output_index},"{index}":0,"delta":{},"sequence_number":{sequence_number}}}"#,
			text.get()
		);
		close_event(&mut self.bytes);
	}

	const fn next_sequence(&mut self) -> u64 {
		self.sequence += 1;
		self.sequence - 1
	}
}

fn event_json(event: &StreamEvent<'_>) -> Vec<u8> {
	match serde_json::to_vec(event) {
		Ok(json) => json,
		Err(error) => unreachable!("events hold only strings, numbers and checked JSON: {error}"),
	}
}

/// Where the items of an empty `output` would go. Only `type`, `id` and `model`
/// come before it, and as JSON strings they can't hold an unescaped quote, so
/// the first match is the field.
fn output_start(json: &[u8]) -> usize {
	const OUTPUT: &[u8] = br#""output":[]"#;
	match memchr::memmem::find(json, OUTPUT) {
		Some(at) => at + OUTPUT.len() - 1,
		None => unreachable!("a response always has `output`"),
	}
}

#[derive(Clone, Copy)]
enum DeltaKind {
	Text,
	Summary,
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
