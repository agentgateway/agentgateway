//! Rewrites decoded reply text and complete tool calls before client encoding.
//! Rewrites work across client protocols and may undo request edits or apply
//! output policy. This module handles JSON escaping.
//!
//! Tool calls are held until they're complete, so a rewrite sees whole arguments
//! and a client never receives half a call. They then go out as a buffered reply
//! sends them: a start and the final arguments, with no deltas. A call the provider
//! cut off goes out unchanged, since its arguments may not be JSON.

use std::fmt;

use serde_json::value::RawValue;

use crate::ir::events::{Arguments, Event, Final, JsonStr, PartId, PartKind};
use crate::json::{decode_string, replace_string_values, string_value};

/// Middleware that changes decoded reply text or complete tool calls.
pub trait ReplyRewrite: Send {
	/// Decoded text for one place in the reply. Write what may go out now; keep
	/// the rest until the next call or `finish`.
	fn text(&mut self, _place: ReplyPlace, text: &str, out: &mut String) {
		out.push_str(text);
	}

	/// The place has no more text. Write anything still kept.
	fn finish(&mut self, _place: ReplyPlace, _out: &mut String) {}

	/// A complete tool call, after its string arguments went through `text`.
	fn call(&mut self, _call: &mut ToolCall) {}
}

/// Where decoded text appears in a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyPlace {
	Answer(PartId),
	Refusal(PartId),
	/// Reasoning summary text the client is shown.
	Reasoning(PartId),
	/// A complete tool call, including its name and arguments.
	ToolCall(PartId),
	/// A string value being rewritten in a tool call's arguments, the `string`th
	/// in order.
	ToolCallArgument {
		part: PartId,
		string: usize,
	},
}

/// A complete tool call being rewritten before client encoding.
pub struct ToolCall {
	pub name: String,
	/// Complete JSON, or empty for no arguments.
	pub arguments: String,
}

/// An ordered reply-rewrite pipeline.
///
/// With no rewrites, events go to the writer untouched. With any, each text delta is
/// decoded and passed through every rewrite, and escaped again only when a rewrite
/// changed it. Tool calls are held until complete and reach the client whole, without
/// their streamed deltas.
#[derive(Default)]
pub struct ReplyRewrites {
	rewrites: Vec<Box<dyn ReplyRewrite>>,
	parts: Vec<OpenPart>,
}

impl fmt::Debug for ReplyRewrites {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "ReplyRewrites({})", self.rewrites.len())
	}
}

struct OpenPart {
	id: PartId,
	kind: OpenKind,
}

enum OpenKind {
	/// Text the client has been sent. For a refusal or reasoning, whose final value
	/// repeats it, `sent` is all of it; an answer's final value has no text.
	Text {
		place: ReplyPlace,
		sent: String,
		had_deltas: bool,
	},
	/// Held until it ends. `arguments` is decoded from the deltas.
	Call {
		call_id: String,
		name: String,
		arguments: String,
	},
}

impl ReplyRewrites {
	/// Each runs in the order given.
	#[must_use]
	pub fn new(rewrites: Vec<Box<dyn ReplyRewrite>>) -> Self {
		Self {
			rewrites,
			parts: Vec::new(),
		}
	}

	pub fn event(&mut self, event: Event<'_>, sink: &mut dyn FnMut(Event<'_>)) {
		if self.rewrites.is_empty() {
			sink(event);
			return;
		}
		match event {
			Event::PartStart { part, kind } => self.part_start(part, kind, event, sink),
			Event::Delta { part, text } => self.delta(part, text, event, sink),
			Event::PartEnd { part, value } => self.part_end(part, value, event, sink),
			_ => sink(event),
		}
	}

	fn part_start(
		&mut self,
		part: PartId,
		kind: PartKind<'_>,
		event: Event<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) {
		let place = match kind {
			PartKind::Text => ReplyPlace::Answer(part),
			PartKind::Refusal => ReplyPlace::Refusal(part),
			PartKind::Reasoning => ReplyPlace::Reasoning(part),
			PartKind::ToolCall { call_id, name } => {
				self.parts.push(OpenPart {
					id: part,
					kind: OpenKind::Call {
						call_id: call_id.to_string(),
						name: name.to_string(),
						arguments: String::new(),
					},
				});
				return;
			},
		};
		self.parts.push(OpenPart {
			id: part,
			kind: OpenKind::Text {
				place,
				sent: String::new(),
				had_deltas: false,
			},
		});
		sink(event);
	}

	fn delta(
		&mut self,
		part: PartId,
		text: JsonStr<'_>,
		event: Event<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) {
		let Some(open) = self.parts.iter_mut().find(|open| open.id == part) else {
			sink(event);
			return;
		};
		match &mut open.kind {
			OpenKind::Call { arguments, .. } => arguments.push_str(&text.decode()),
			OpenKind::Text {
				place,
				sent,
				had_deltas,
			} => {
				*had_deltas = true;
				let decoded = text.decode();
				let out = through(&mut self.rewrites, *place, &decoded, false);
				if out == decoded {
					sink(event);
				} else {
					send_text(part, &out, sink);
				}
				if !matches!(place, ReplyPlace::Answer(_)) {
					sent.push_str(&out);
				}
			},
		}
	}

	fn part_end(
		&mut self,
		part: PartId,
		value: Final<'_>,
		event: Event<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) {
		let Some(at) = self.parts.iter().position(|open| open.id == part) else {
			sink(event);
			return;
		};
		match self.parts.remove(at).kind {
			OpenKind::Call {
				call_id,
				name,
				arguments,
			} => {
				let Final::ToolCall(sent) = value else {
					unreachable!("a tool call part ends with its arguments");
				};
				self.call_end(part, &call_id, name, arguments, sent, sink);
			},
			OpenKind::Text {
				place,
				mut sent,
				had_deltas,
			} => {
				// A part with no deltas sends its text only in its final value.
				let whole = match value {
					Final::Refusal(text) => Some(text),
					Final::Reasoning { text, .. } => text,
					_ => None,
				}
				.filter(|_| !had_deltas);
				let whole_text = whole.map(|text| text.decode());
				let rest = through(
					&mut self.rewrites,
					place,
					whole_text.as_deref().unwrap_or(""),
					true,
				);
				if whole.is_none() {
					send_text(part, &rest, sink);
				}
				sent.push_str(&rest);
				end_text(part, value, &sent, sink);
			},
		}
	}

	fn call_end(
		&mut self,
		part: PartId,
		call_id: &str,
		name: String,
		streamed: String,
		sent: Arguments<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) {
		let Arguments::Complete(_) = sent else {
			// The provider stopped inside the call, so its arguments may not be JSON.
			sink(Event::PartStart {
				part,
				kind: PartKind::ToolCall {
					call_id,
					name: &name,
				},
			});
			sink(Event::PartEnd {
				part,
				value: Final::ToolCall(sent),
			});
			return;
		};
		let arguments = if streamed.is_empty() {
			decode_string(sent.raw()).into_owned()
		} else {
			streamed
		};
		let rewritten = replace_string_values(&arguments, |string, text| {
			let place = ReplyPlace::ToolCallArgument { part, string };
			let out = through(&mut self.rewrites, place, &text, true);
			(out != text).then_some(out)
		});
		let mut call = ToolCall {
			name,
			arguments: rewritten.unwrap_or(arguments),
		};
		for rewrite in &mut self.rewrites {
			rewrite.call(&mut call);
		}
		send_call(part, call_id, &call, sink);
	}
}

fn send_call(part: PartId, call_id: &str, call: &ToolCall, sink: &mut dyn FnMut(Event<'_>)) {
	// A tool call part carries its arguments as one JSON string.
	let raw = escaped_raw(&call.arguments);
	let text = json_str(&raw);
	sink(Event::PartStart {
		part,
		kind: PartKind::ToolCall {
			call_id,
			name: &call.name,
		},
	});
	sink(Event::PartEnd {
		part,
		value: Final::ToolCall(Arguments::complete(text).unwrap_or(Arguments::Partial(text))),
	});
}

/// Passes `text` through every rewrite, and when `finish`, what each still keeps.
fn through(
	rewrites: &mut [Box<dyn ReplyRewrite>],
	place: ReplyPlace,
	text: &str,
	finish: bool,
) -> String {
	let mut carry = text.to_string();
	for rewrite in rewrites.iter_mut() {
		let mut out = String::new();
		if !carry.is_empty() {
			rewrite.text(place, &carry, &mut out);
		}
		if finish {
			rewrite.finish(place, &mut out);
		}
		carry = out;
	}
	carry
}

fn send_text(part: PartId, text: &str, sink: &mut dyn FnMut(Event<'_>)) {
	if text.is_empty() {
		return;
	}
	let raw = escaped_raw(text);
	sink(Event::Delta {
		part,
		text: json_str(&raw),
	});
}

fn end_text(part: PartId, value: Final<'_>, sent: &str, sink: &mut dyn FnMut(Event<'_>)) {
	let raw;
	let value = match value {
		Final::Refusal(_) => {
			raw = escaped_raw(sent);
			Final::Refusal(json_str(&raw))
		},
		Final::Reasoning {
			text: Some(_),
			encrypted,
			item,
		} => {
			raw = escaped_raw(sent);
			Final::Reasoning {
				text: Some(json_str(&raw)),
				encrypted,
				item,
			}
		},
		other => other,
	};
	sink(Event::PartEnd { part, value });
}

pub(super) fn escaped_raw(text: &str) -> Box<RawValue> {
	match string_value(text) {
		Ok(raw) => raw,
		Err(error) => unreachable!("a string always serializes: {error}"),
	}
}

pub(super) fn json_str(raw: &RawValue) -> JsonStr<'_> {
	match JsonStr::new(raw) {
		Some(text) => text,
		None => unreachable!("escaped text is a JSON string"),
	}
}

#[cfg(test)]
#[path = "rewrite_tests.rs"]
mod tests;
