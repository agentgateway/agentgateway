//! Stop sequences for a provider that can't take them. This sits between a
//! decoder and the client's writer, and ends the answer at the first match.
//!
//! The provider's stream is still read to the end, because its usage comes last.
//! Matches stay within each text part. With overlapping parts, the first observed
//! match closes all open text parts; non-text parts already started still finish.

use crate::ir::events::{Event, Final, JsonStr, Outcome, PartId, PartKind};
use crate::json::string_value;

/// Removes reply text after the first emulated stop-sequence match.
pub struct StopAt {
	stops: Vec<String>,
	/// Text parts and their possible stop-sequence prefixes, in start order.
	text: Vec<(PartId, String)>,
	/// The sequence that ended the answer. Nothing after it is the client's.
	matched: Option<String>,
	/// Parts started and not yet ended. Those open at the match still finish,
	/// because the client already saw them start.
	open: Vec<PartId>,
}

impl StopAt {
	/// See [`crate::ReplyPlan::emulated_stop_sequences`].
	#[must_use]
	pub fn new(sequences: Vec<String>) -> Self {
		Self {
			stops: sequences
				.into_iter()
				.filter(|stop| !stop.is_empty())
				.collect(),
			text: Vec::new(),
			matched: None,
			open: Vec::new(),
		}
	}

	pub fn event(&mut self, event: Event<'_>, sink: &mut dyn FnMut(Event<'_>)) {
		if self.stops.is_empty() {
			sink(event);
			return;
		}
		if let Some(stop) = &self.matched {
			match event {
				Event::ServiceTier(_) | Event::Usage(_) | Event::Error(_) => sink(event),
				Event::Delta { part, .. } | Event::Citation { part, .. } if self.open.contains(&part) => {
					sink(event);
				},
				Event::PartEnd { part, .. } if self.open.contains(&part) => {
					self.open.retain(|open| *open != part);
					sink(event);
				},
				// The provider's ending is about text the client never saw.
				Event::Finish(_) => {
					sink(Event::StopSequence(stop));
					sink(Event::Finish(Outcome::Completed));
				},
				_ => {},
			}
			return;
		}
		match event {
			Event::PartStart { part, .. } => self.open.push(part),
			Event::PartEnd { part, .. } => self.open.retain(|open| *open != part),
			_ => {},
		}
		match event {
			Event::PartStart {
				part,
				kind: PartKind::Text,
			} => {
				self.text.push((part, String::new()));
				sink(event);
			},
			Event::Delta { part, text } => {
				if let Some(at) = self.text.iter().position(|(id, _)| *id == part) {
					self.text_delta(at, &text.decode(), sink);
				} else {
					sink(event);
				}
			},
			Event::PartEnd { part, .. } => {
				if let Some(at) = self.text.iter().position(|(id, _)| *id == part) {
					// A prefix that never matched is answer text after all.
					let (_, held) = self.text.remove(at);
					send(part, &held, sink);
				}
				sink(event);
			},
			_ => sink(event),
		}
	}

	fn text_delta(&mut self, at: usize, text: &str, sink: &mut dyn FnMut(Event<'_>)) {
		let (part, held) = &mut self.text[at];
		held.push_str(text);
		if let Some((offset, stop)) = first_stop(held, &self.stops) {
			send(*part, &held[..offset], sink);
			self.matched = Some(stop.to_string());
			held.clear();
			for (part, held) in self.text.drain(..) {
				send(part, &held, sink);
				sink(Event::PartEnd {
					part,
					value: Final::Text,
				});
				self.open.retain(|open| *open != part);
			}
			return;
		}
		let ready = held.len() - held_back(held, &self.stops);
		send(*part, &held[..ready], sink);
		held.drain(..ready);
	}
}

#[expect(clippy::expect_used, reason = "escaped text is always a JSON string")]
fn send(part: PartId, text: &str, sink: &mut dyn FnMut(Event<'_>)) {
	if text.is_empty() {
		return;
	}
	let raw = string_value(text).expect("escaped text is JSON");
	let text = JsonStr::new(&raw).expect("escaped text is a JSON string");
	sink(Event::Delta { part, text });
}

/// The earliest match, so a later sequence can't let earlier text through.
fn first_stop<'s>(text: &str, stops: &'s [String]) -> Option<(usize, &'s str)> {
	stops
		.iter()
		.filter_map(|stop| text.find(stop.as_str()).map(|at| (at, stop.as_str())))
		.min_by_key(|(at, _)| *at)
}

/// How many trailing bytes could still become a stop sequence with more text.
fn held_back(text: &str, stops: &[String]) -> usize {
	let longest = stops.iter().map(String::len).max().unwrap_or(0);
	(1..longest.min(text.len() + 1))
		.rev()
		.filter(|len| text.is_char_boundary(text.len() - len))
		.find(|len| {
			let tail = &text[text.len() - len..];
			stops.iter().any(|stop| stop.starts_with(tail))
		})
		.unwrap_or(0)
}

#[cfg(test)]
#[path = "stop_tests.rs"]
mod tests;
