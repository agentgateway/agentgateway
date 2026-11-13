//! Holds reply output back until the caller has checked it.
//!
//! This sits between the reply rewrites and the client's writer, so the caller checks
//! what the client would see. Each text part's deltas gather into windows. A full window,
//! or what is left when its part ends, is offered to the caller, and that output waits
//! for the verdict; a part's start waits with its first window. A complete tool call is
//! offered whole and waits too. While a check is
//! pending, later events are kept in order, up to a byte limit, and replayed when the
//! caller releases the reply. Waiting, time limits and failure modes are the caller's.

use std::mem;

use serde_json::json;
use serde_json::value::RawValue;

use super::rewrite::ReplyPlace;
use crate::ir::events::{
	Arguments, CompleteArguments, ErrorKind, Event, Final, JsonStr, Outcome, PartId, PartKind,
	ProviderError, StopReason, Usage,
};
use crate::json::decode_string;

/// How reply output is held back for the caller to check.
#[derive(Debug, Clone)]
pub struct OutputChecks {
	/// Text to gather for a part, in bytes, before offering it for checking.
	pub window_bytes: usize,
	/// Bytes from the end of a part's previous window to offer again at the start of
	/// the next, as context. The client is sent them once.
	pub overlap_bytes: usize,
	/// The most provider output kept back at once. Past it the reply fails.
	pub max_held_bytes: usize,
	pub incomplete_call: IncompleteCall,
}

/// What to do with a tool call the provider ended before its arguments were complete.
/// Its arguments may not be JSON, so it can't be offered for checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncompleteCall {
	/// End the reply as `Verdict::Stop` with this message does.
	Stop(String),
	/// Leave the call out of the reply.
	Drop,
	/// Send the call unchecked.
	Send,
}

/// Output waiting for the caller's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Check<'a> {
	pub place: ReplyPlace,
	/// For text, any overlap and then the new text. For a tool call, its name and
	/// arguments as a JSON object.
	pub text: &'a str,
}

/// What to do with output waiting for a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
	Release,
	/// Fail the reply with this message, as each client protocol says an invalid request:
	/// an HTTP error if the client was sent nothing yet, else an error that ends its
	/// stream. What was held is never sent. Accounting goes on to the provider's end.
	Stop(String),
}

/// Holds output back for [`OutputChecks`].
pub(crate) struct Checks {
	config: OutputChecks,
	/// Parts whose output is being gathered, in start order.
	gathering: Vec<Gathering>,
	waiting: Option<Waiting>,
	/// Events that arrived while waiting, in order.
	queue: Vec<HeldEvent>,
	/// Provider bytes in `gathering`, `waiting` and `queue`.
	held_bytes: usize,
	state: State,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
	Running,
	Stopped,
	/// Held output passed `max_held_bytes`.
	Failed,
}

struct Gathering {
	part: PartId,
	kind: Gathered,
	/// The part's events since its last check, to send when released.
	events: Vec<HeldEvent>,
}

enum Gathered {
	Text {
		place: ReplyPlace,
		/// Decoded text since the last check.
		text: String,
		/// The end of the last window, offered again with the next.
		overlap: String,
		had_deltas: bool,
	},
	Call {
		name: String,
	},
}

struct Waiting {
	place: ReplyPlace,
	text: String,
	release: Vec<HeldEvent>,
}

impl Checks {
	pub(crate) const fn new(config: OutputChecks) -> Self {
		Self {
			config,
			gathering: Vec::new(),
			waiting: None,
			queue: Vec::new(),
			held_bytes: 0,
			state: State::Running,
		}
	}

	pub(crate) fn event(&mut self, event: Event<'_>, sink: &mut dyn FnMut(Event<'_>)) {
		if self.state != State::Running {
			return;
		}
		if self.waiting.is_some() {
			let held = self.keep(event);
			self.queue.push(held);
			self.check_bound();
			return;
		}
		match event {
			Event::PartStart { part, kind } => self.part_start(part, kind, event),
			Event::Delta { part, text } => self.part_output(part, Some(text), event, sink),
			Event::Citation { part, .. } => self.part_output(part, None, event, sink),
			Event::PartEnd { part, value } => self.part_end(part, value, event, sink),
			_ => sink(event),
		}
		self.check_bound();
	}

	/// What the reply waits for, if anything.
	pub(crate) fn waiting(&self) -> Option<Check<'_>> {
		self.waiting.as_ref().map(|waiting| Check {
			place: waiting.place,
			text: &waiting.text,
		})
	}

	/// Whether held output passed its limit, which fails the reply.
	pub(crate) fn failed(&self) -> bool {
		self.state == State::Failed
	}

	pub(crate) fn resolve(&mut self, verdict: Verdict, sink: &mut dyn FnMut(Event<'_>)) {
		let Some(waiting) = self.waiting.take() else {
			unreachable!("only a waiting reply can be resolved");
		};
		match verdict {
			Verdict::Release => {
				for held in waiting.release {
					self.held_bytes -= held.size();
					sink(held.as_event());
				}
			},
			Verdict::Stop(message) => self.stop(&message, sink),
		}
		for held in mem::take(&mut self.queue) {
			self.held_bytes -= held.size();
			self.event(held.as_event(), sink);
		}
	}

	/// A part's start waits with its first window, so a client that some writers would
	/// already have sent a reply's start gets nothing before the first verdict, and a stop
	/// on it can still be an HTTP error.
	fn part_start(&mut self, part: PartId, kind: PartKind<'_>, event: Event<'_>) {
		let text = |place| Gathered::Text {
			place,
			text: String::new(),
			overlap: String::new(),
			had_deltas: false,
		};
		let kind = match kind {
			PartKind::Text => text(ReplyPlace::Answer(part)),
			PartKind::Refusal => text(ReplyPlace::Refusal(part)),
			PartKind::Reasoning => text(ReplyPlace::Reasoning(part)),
			PartKind::ToolCall { name, .. } => Gathered::Call {
				name: name.to_string(),
			},
		};
		let held = self.keep(event);
		self.gathering.push(Gathering {
			part,
			kind,
			events: vec![held],
		});
	}

	fn part_output(
		&mut self,
		part: PartId,
		delta: Option<JsonStr<'_>>,
		event: Event<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) {
		let Some(at) = self.gathering_at(part) else {
			sink(event);
			return;
		};
		let held = self.keep(event);
		let gathering = &mut self.gathering[at];
		gathering.events.push(held);
		let full = match (&mut gathering.kind, delta) {
			(
				Gathered::Text {
					text, had_deltas, ..
				},
				Some(delta),
			) => {
				*had_deltas = true;
				text.push_str(&delta.decode());
				text.len() >= self.config.window_bytes
			},
			_ => false,
		};
		if full {
			let (place, text, events) = window(&mut self.gathering[at], self.config.overlap_bytes);
			self.wait(place, text, events);
		}
	}

	fn part_end(
		&mut self,
		part: PartId,
		value: Final<'_>,
		event: Event<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) {
		let Some(at) = self.gathering_at(part) else {
			sink(event);
			return;
		};
		let held = self.keep(event);
		let mut gathering = self.gathering.remove(at);
		gathering.events.push(held);
		match &mut gathering.kind {
			Gathered::Text {
				text, had_deltas, ..
			} => {
				// A part with no deltas has its text only in its final value.
				let whole = match value {
					Final::Refusal(whole) => Some(whole),
					Final::Reasoning { text, .. } => text,
					_ => None,
				};
				if let Some(whole) = whole.filter(|_| !*had_deltas) {
					text.push_str(&whole.decode());
				}
				if text.is_empty() {
					self.release(gathering.events, sink);
				} else {
					let (place, text, events) = window(&mut gathering, self.config.overlap_bytes);
					self.wait(place, text, events);
				}
			},
			Gathered::Call { name } => {
				let Final::ToolCall(arguments) = value else {
					unreachable!("a tool call part ends with its arguments");
				};
				if arguments.is_complete() {
					let text = json!({
						"name": name,
						"arguments": decode_string(arguments.raw()),
					})
					.to_string();
					self.wait(ReplyPlace::ToolCall(part), text, gathering.events);
					return;
				}
				match self.config.incomplete_call.clone() {
					IncompleteCall::Stop(message) => {
						self.drop_held(gathering.events);
						self.stop(&message, sink);
					},
					IncompleteCall::Drop => self.drop_held(gathering.events),
					IncompleteCall::Send => self.release(gathering.events, sink),
				}
			},
		}
	}

	fn wait(&mut self, place: ReplyPlace, text: String, release: Vec<HeldEvent>) {
		assert!(self.waiting.is_none(), "one check waits at a time");
		self.waiting = Some(Waiting {
			place,
			text,
			release,
		});
	}

	/// Sends held events the caller needn't check.
	fn release(&mut self, events: Vec<HeldEvent>, sink: &mut dyn FnMut(Event<'_>)) {
		for held in events {
			self.held_bytes -= held.size();
			sink(held.as_event());
		}
	}

	fn drop_held(&mut self, events: Vec<HeldEvent>) {
		self.held_bytes -= events.into_iter().map(|event| event.size()).sum::<usize>();
	}

	fn gathering_at(&self, part: PartId) -> Option<usize> {
		self
			.gathering
			.iter()
			.position(|gathering| gathering.part == part)
	}

	fn keep(&mut self, event: Event<'_>) -> HeldEvent {
		let held = HeldEvent::new(event);
		self.held_bytes += held.size();
		held
	}

	fn check_bound(&mut self) {
		if self.held_bytes > self.config.max_held_bytes {
			self.state = State::Failed;
			self.gathering.clear();
			self.waiting = None;
			self.queue.clear();
			self.held_bytes = 0;
		}
	}

	/// Fails the reply with the message and drops everything held. Nothing after this
	/// reaches the client: accounting reads the provider's events before they get here.
	fn stop(&mut self, message: &str, sink: &mut dyn FnMut(Event<'_>)) {
		self.state = State::Stopped;
		self.waiting = None;
		self.gathering.clear();
		self.queue.clear();
		self.held_bytes = 0;
		sink(Event::Error(ProviderError {
			kind: ErrorKind::InvalidRequest,
			code: None,
			message: None,
			client_message: Some(message),
		}));
	}
}

/// A text part's gathered text after the last window's overlap, and the events to send
/// once it is released.
fn window(gathering: &mut Gathering, overlap_bytes: usize) -> (ReplyPlace, String, Vec<HeldEvent>) {
	let Gathered::Text {
		place,
		text,
		overlap,
		..
	} = &mut gathering.kind
	else {
		unreachable!("only text gathers into windows");
	};
	let mut checked = mem::take(overlap);
	checked.push_str(text);
	text.clear();
	*overlap = suffix(&checked, overlap_bytes).to_string();
	(*place, checked, mem::take(&mut gathering.events))
}

fn suffix(text: &str, max_bytes: usize) -> &str {
	let mut start = text.len().saturating_sub(max_bytes);
	while !text.is_char_boundary(start) {
		start += 1;
	}
	&text[start..]
}

/// An event copied out of the decoder's buffer, to send after a check.
enum HeldEvent {
	ResponseStart {
		id: String,
		model: String,
	},
	PartStart {
		part: PartId,
		kind: HeldKind,
	},
	Delta {
		part: PartId,
		text: Box<RawValue>,
	},
	PartEnd {
		part: PartId,
		value: HeldFinal,
	},
	Citation {
		part: PartId,
		url: String,
		title: Option<String>,
	},
	MessageEnd {
		item: Box<RawValue>,
	},
	ServiceTier(String),
	StopSequence(String),
	Usage(Usage),
	Finish(Option<(StopReason, Option<String>)>),
	Error {
		kind: ErrorKind,
		code: Option<String>,
		message: Option<String>,
		client_message: Option<String>,
	},
}

enum HeldKind {
	Text,
	Refusal,
	Reasoning,
	ToolCall { call_id: String, name: String },
}

enum HeldFinal {
	Text,
	Refusal(Box<RawValue>),
	ToolCall {
		complete: bool,
		raw: Box<RawValue>,
	},
	Reasoning {
		text: Option<Box<RawValue>>,
		encrypted: Option<Box<RawValue>>,
		item: Box<RawValue>,
	},
}

impl HeldEvent {
	fn new(event: Event<'_>) -> Self {
		let raw = |raw: &RawValue| raw.to_owned();
		match event {
			Event::ResponseStart { id, model } => Self::ResponseStart {
				id: id.to_string(),
				model: model.to_string(),
			},
			Event::PartStart { part, kind } => Self::PartStart {
				part,
				kind: match kind {
					PartKind::Text => HeldKind::Text,
					PartKind::Refusal => HeldKind::Refusal,
					PartKind::Reasoning => HeldKind::Reasoning,
					PartKind::ToolCall { call_id, name } => HeldKind::ToolCall {
						call_id: call_id.to_string(),
						name: name.to_string(),
					},
				},
			},
			Event::Delta { part, text } => Self::Delta {
				part,
				text: raw(text.raw_value()),
			},
			Event::PartEnd { part, value } => Self::PartEnd {
				part,
				value: match value {
					Final::Text => HeldFinal::Text,
					Final::Refusal(text) => HeldFinal::Refusal(raw(text.raw_value())),
					Final::ToolCall(arguments) => HeldFinal::ToolCall {
						complete: arguments.is_complete(),
						raw: raw(arguments.raw_value()),
					},
					Final::Reasoning {
						text,
						encrypted,
						item,
					} => HeldFinal::Reasoning {
						text: text.map(|text| raw(text.raw_value())),
						encrypted: encrypted.map(|text| raw(text.raw_value())),
						item: raw(item),
					},
				},
			},
			Event::Citation { part, url, title } => Self::Citation {
				part,
				url: url.to_string(),
				title: title.map(str::to_string),
			},
			Event::MessageEnd { item } => Self::MessageEnd { item: raw(item) },
			Event::ServiceTier(tier) => Self::ServiceTier(tier.to_string()),
			Event::StopSequence(stop) => Self::StopSequence(stop.to_string()),
			Event::Usage(usage) => Self::Usage(usage),
			Event::Finish(Outcome::Completed) => Self::Finish(None),
			Event::Finish(Outcome::Incomplete { reason, raw }) => {
				Self::Finish(Some((reason, raw.map(str::to_string))))
			},
			Event::Error(ProviderError {
				kind,
				code,
				message,
				client_message,
			}) => Self::Error {
				kind,
				code: code.map(str::to_string),
				message: message.map(str::to_string),
				client_message: client_message.map(str::to_string),
			},
		}
	}

	/// The provider bytes this keeps, counted against `max_held_bytes`.
	fn size(&self) -> usize {
		let raw = |raw: &RawValue| raw.get().len();
		let text = |text: &Option<String>| text.as_ref().map_or(0, String::len);
		match self {
			Self::ResponseStart { id, model } => id.len() + model.len(),
			Self::PartStart { kind, .. } => match kind {
				HeldKind::ToolCall { call_id, name } => call_id.len() + name.len(),
				_ => 0,
			},
			Self::Delta { text, .. } => raw(text),
			Self::PartEnd { value, .. } => match value {
				HeldFinal::Text => 0,
				HeldFinal::Refusal(text) => raw(text),
				HeldFinal::ToolCall { raw: text, .. } => raw(text),
				HeldFinal::Reasoning {
					text,
					encrypted,
					item,
				} => text.as_deref().map_or(0, raw) + encrypted.as_deref().map_or(0, raw) + raw(item),
			},
			Self::Citation { url, title, .. } => url.len() + text(title),
			Self::MessageEnd { item } => raw(item),
			Self::ServiceTier(text) | Self::StopSequence(text) => text.len(),
			Self::Usage(_) => 0,
			Self::Finish(finish) => finish
				.as_ref()
				.map_or(0, |(_, raw)| raw.as_ref().map_or(0, String::len)),
			Self::Error {
				code,
				message,
				client_message,
				..
			} => text(code) + text(message) + text(client_message),
		}
	}

	/// The copies were checked when they were first read, so they're used as they are.
	fn as_event(&self) -> Event<'_> {
		match self {
			Self::ResponseStart { id, model } => Event::ResponseStart { id, model },
			Self::PartStart { part, kind } => Event::PartStart {
				part: *part,
				kind: match kind {
					HeldKind::Text => PartKind::Text,
					HeldKind::Refusal => PartKind::Refusal,
					HeldKind::Reasoning => PartKind::Reasoning,
					HeldKind::ToolCall { call_id, name } => PartKind::ToolCall { call_id, name },
				},
			},
			Self::Delta { part, text } => Event::Delta {
				part: *part,
				text: JsonStr(text),
			},
			Self::PartEnd { part, value } => Event::PartEnd {
				part: *part,
				value: match value {
					HeldFinal::Text => Final::Text,
					HeldFinal::Refusal(text) => Final::Refusal(JsonStr(text)),
					HeldFinal::ToolCall {
						complete: true,
						raw,
					} => Final::ToolCall(Arguments::Complete(CompleteArguments(JsonStr(raw)))),
					HeldFinal::ToolCall {
						complete: false,
						raw,
					} => Final::ToolCall(Arguments::Partial(JsonStr(raw))),
					HeldFinal::Reasoning {
						text,
						encrypted,
						item,
					} => Final::Reasoning {
						text: text.as_deref().map(JsonStr),
						encrypted: encrypted.as_deref().map(JsonStr),
						item,
					},
				},
			},
			Self::Citation { part, url, title } => Event::Citation {
				part: *part,
				url,
				title: title.as_deref(),
			},
			Self::MessageEnd { item } => Event::MessageEnd { item },
			Self::ServiceTier(tier) => Event::ServiceTier(tier),
			Self::StopSequence(stop) => Event::StopSequence(stop),
			Self::Usage(usage) => Event::Usage(*usage),
			Self::Finish(None) => Event::Finish(Outcome::Completed),
			Self::Finish(Some((reason, raw))) => Event::Finish(Outcome::Incomplete {
				reason: *reason,
				raw: raw.as_deref(),
			}),
			Self::Error {
				kind,
				code,
				message,
				client_message,
			} => Event::Error(ProviderError {
				kind: *kind,
				code: code.as_deref(),
				message: message.as_deref(),
				client_message: client_message.as_deref(),
			}),
		}
	}
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;
