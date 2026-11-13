//! Strict on purpose: an unknown, repeated or out-of-order event is an error, so a
//! broken stream never turns into a plausible answer.
//!
//! Deltas aren't kept. A running length and hash per part is enough to check them
//! against the final value, so memory doesn't grow with the answer.
//!
//! Shared by a whole terminal event, one scanned in pieces, and a buffered body.

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::value::RawValue;

use super::terminal_scan::TerminalScanner;
use super::wire::{
	Wire, WireError, WireIncomplete, WireItem, WirePart, WireStart, WireTerminal, WireUsage, string,
	summary_text,
};
use crate::ir::events::{
	Arguments, DecodeError, ErrorKind, Event, Final, JsonStr, MAX_PARTS, Outcome, PartId, PartIds,
	PartKind, ProviderError, StopReason, Usage,
};
use crate::protocols::openai_error_field;

const DONE: &[u8] = b"[DONE]";
#[derive(Default)]
pub struct ResponsesDecoder {
	started: bool,
	stage: Stage,
	parts: PartIds,
	items: Vec<Item>,
	pieces: Pieces,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Stage {
	#[default]
	Reading,
	/// The stream broke. The client has its failure, and only usage is still read.
	Draining,
	Finished,
}

/// An event over the caller's limit, which arrives in pieces.
#[derive(Default)]
enum Pieces {
	#[default]
	None,
	Scanning(Box<TerminalScanner>),
	/// The rest of an event that can't be scanned.
	Skipping,
}

impl ResponsesDecoder {
	/// Stop reading the provider once this is true.
	#[must_use]
	pub fn is_finished(&self) -> bool {
		self.stage == Stage::Finished
	}

	/// Takes one event payload after the caller removes SSE framing.
	/// The caller bounds complete payloads; oversized terminal events use
	/// `push_json_piece` and `end_json_pieces` instead.
	///
	/// After an error the stream is broken and must stay failed, but tokens it
	/// spent still count. Keep calling until `is_finished` or the input ends:
	/// the only events that can follow are `ServiceTier` and `Usage`.
	pub fn push_json(
		&mut self,
		json: &[u8],
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if json == DONE {
			return self.done_line();
		}
		match self.stage {
			Stage::Finished => Ok(()),
			Stage::Draining => {
				self.drain_json(json, sink);
				Ok(())
			},
			Stage::Reading => {
				let pushed = self.push(json, sink);
				self.drain_after(pushed)
			},
		}
	}

	/// Only a terminal event may arrive in pieces. The pieces aren't kept.
	pub fn push_json_piece(
		&mut self,
		piece: &[u8],
		max_event_bytes: usize,
	) -> Result<(), DecodeError> {
		if self.is_finished() || matches!(self.pieces, Pieces::Skipping) {
			return Ok(());
		}
		let mut scanner = match std::mem::take(&mut self.pieces) {
			Pieces::Scanning(scanner) => scanner,
			_ => Box::new(TerminalScanner::new(max_event_bytes)),
		};
		let pushed = scanner.push(piece);
		self.pieces = match pushed {
			Ok(()) => Pieces::Scanning(scanner),
			Err(_) => Pieces::Skipping,
		};
		match self.stage {
			Stage::Draining => Ok(()),
			_ => self.drain_after(pushed),
		}
	}

	pub fn end_json_pieces(
		&mut self,
		max_event_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		match self.stage {
			Stage::Finished => Ok(()),
			Stage::Draining => {
				self.drain_pieces_end(sink);
				Ok(())
			},
			Stage::Reading => {
				let ended = self.end_pieces(max_event_bytes, sink);
				self.drain_after(ended)
			},
		}
	}

	/// An error while reading starts draining, unless the event also finished the reply.
	fn drain_after(&mut self, result: Result<(), DecodeError>) -> Result<(), DecodeError> {
		if result.is_err() && self.stage == Stage::Reading {
			self.stage = Stage::Draining;
		}
		result
	}

	fn done_line(&mut self) -> Result<(), DecodeError> {
		if self.stage != Stage::Reading {
			return Ok(());
		}
		self.stage = Stage::Draining;
		Err(DecodeError::NoTerminalEvent)
	}

	fn end_pieces(
		&mut self,
		max_event_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		// An event with empty data sends no piece, or an empty one.
		let scanner = match std::mem::take(&mut self.pieces) {
			Pieces::Scanning(scanner) => scanner,
			_ => Box::new(TerminalScanner::new(max_event_bytes)),
		};
		let scanned = scanner.finish()?;
		self.require_started()?;
		let kind = scanned.kind.ok_or(DecodeError::WrongType("type"))?;
		self.end_response(kind, &scanned.record(), sink)
	}

	/// Nothing here is trusted: anything unreadable is passed over.
	fn drain_json(&mut self, json: &[u8], sink: &mut dyn FnMut(Event<'_>)) {
		let Ok(wire) = serde_json::from_slice::<Wire<'_>>(json) else {
			return;
		};
		let is_terminal = Terminal::from_event(&wire.kind).is_some();
		let response = wire.response.filter(|_| is_terminal);
		let Some(response) = response else {
			return;
		};
		let Ok(terminal) = serde_json::from_str::<WireTerminal<'_>>(response.get()) else {
			return;
		};
		self.stage = Stage::Finished;
		if let Some(tier) = terminal.service_tier.as_deref() {
			sink(Event::ServiceTier(tier));
		}
		if let Some(usage) = terminal.usage {
			sink(Event::Usage(usage.into()));
		}
	}

	fn drain_pieces_end(&mut self, sink: &mut dyn FnMut(Event<'_>)) {
		let Pieces::Scanning(scanner) = std::mem::take(&mut self.pieces) else {
			return;
		};
		let Ok(scanned) = scanner.finish() else {
			return;
		};
		self.stage = Stage::Finished;
		if let Some(tier) = scanned.service_tier.as_deref() {
			sink(Event::ServiceTier(tier));
		}
		if let Some(usage) = scanned.usage {
			sink(Event::Usage(usage));
		}
	}

	fn push(&mut self, json: &[u8], sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
		let wire: Wire<'_> = serde_json::from_slice(json)?;
		match wire.kind.as_ref() {
			"response.created" => self.created(&wire, sink),
			"response.in_progress" | "response.queued" | "keepalive" | "response.metadata" => Ok(()),
			"response.output_item.added" => self.item_added(&wire, sink),
			"response.output_item.done" => self.item_done(&wire, sink),
			"response.content_part.added" => self.part_added(&wire, sink),
			"response.output_text.delta" => self.text_delta(&wire, TextKind::Text, sink),
			"response.output_text.annotation.added" => self.annotation(&wire, sink),
			"response.refusal.delta" => self.text_delta(&wire, TextKind::Refusal, sink),
			"response.output_text.done" => self.text_done(
				wire.output_index,
				wire.content_index,
				TextKind::Text,
				wire.text,
				sink,
			),
			"response.refusal.done" => {
				let text = wire.refusal;
				self.text_done(
					wire.output_index,
					wire.content_index,
					TextKind::Refusal,
					text,
					sink,
				)
			},
			"response.function_call_arguments.delta" => self.arguments_delta(&wire, sink),
			"response.function_call_arguments.done" => self.arguments_done(&wire),
			"response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
				self.reasoning_delta(&wire, sink)
			},
			// Their text arrives through deltas and the completed item.
			"response.content_part.done"
			| "response.reasoning_summary_part.added"
			| "response.reasoning_summary_part.done"
			| "response.reasoning_summary_text.done"
			| "response.reasoning_text.done" => self.require_started(),
			"response.completed" => self.terminal(&wire, Terminal::Completed, sink),
			"response.incomplete" => self.terminal(&wire, Terminal::Incomplete, sink),
			"response.failed" => self.terminal(&wire, Terminal::Failed, sink),
			"error" => {
				self.stage = Stage::Finished;
				let nested = wire.error.as_ref();
				let nested_code = nested.and_then(|error| error.code.as_deref());
				let nested_message = nested.and_then(|error| error.message.as_deref());
				sink(Event::Error(provider_failure(
					wire.code.as_deref().or(nested_code),
					wire.message.as_deref().or(nested_message),
				)));
				Ok(())
			},
			other => Err(DecodeError::UnknownEvent(other.to_string())),
		}
	}

	/// Call when input ends.
	pub fn finish(&self) -> Result<(), DecodeError> {
		if self.is_finished() {
			Ok(())
		} else {
			Err(DecodeError::NoTerminalEvent)
		}
	}

	const fn require_started(&self) -> Result<(), DecodeError> {
		if self.started {
			Ok(())
		} else {
			Err(DecodeError::OutOfOrder("event before response.created"))
		}
	}

	fn created(
		&mut self,
		wire: &Wire<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if self.started {
			return Err(DecodeError::OutOfOrder("repeated response.created"));
		}
		self.started = true;
		let response = wire.response.ok_or(DecodeError::WrongType("response"))?;
		let start: WireStart<'_> = serde_json::from_str(response.get())?;
		sink(Event::ResponseStart {
			id: &start.id,
			model: &start.model,
		});
		Ok(())
	}

	fn item_added(
		&mut self,
		wire: &Wire<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		self.require_started()?;
		if self.items.len() >= MAX_PARTS {
			return Err(DecodeError::TooManyParts);
		}
		let output_index = wire
			.output_index
			.ok_or(DecodeError::WrongType("output_index"))?;
		if self
			.items
			.iter()
			.any(|item| item.output_index == output_index)
		{
			return Err(DecodeError::OutOfOrder("repeated output_item.added"));
		}
		let raw = wire.item.ok_or(DecodeError::WrongType("item"))?;
		let item: WireItem<'_> = serde_json::from_str(raw.get())?;
		let state = match item.kind.as_ref() {
			"message" => ItemState::Message { parts: Vec::new() },
			"function_call" => {
				let call_id = item.call_id.ok_or(DecodeError::WrongType("call_id"))?;
				let name = item.name.ok_or(DecodeError::WrongType("name"))?;
				let id = self.parts.allocate()?;
				sink(Event::PartStart {
					part: id,
					kind: PartKind::ToolCall {
						call_id: &call_id,
						name: &name,
					},
				});
				ItemState::ToolCall {
					id,
					streamed: Check::EMPTY,
					arguments_done: false,
				}
			},
			"reasoning" => {
				let id = self.parts.allocate()?;
				sink(Event::PartStart {
					part: id,
					kind: PartKind::Reasoning,
				});
				ItemState::Reasoning {
					id,
					encrypted: false,
					streamed_summary: None,
				}
			},
			other => return Err(DecodeError::UnsupportedItem(other.to_string())),
		};
		self.items.push(Item {
			output_index,
			open: true,
			state,
		});
		Ok(())
	}

	fn part_added(
		&mut self,
		wire: &Wire<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let raw = wire.part.ok_or(DecodeError::WrongType("part"))?;
		let part: WirePart<'_> = serde_json::from_str(raw.get())?;
		let kind = match part.kind.as_ref() {
			"output_text" => TextKind::Text,
			"refusal" => TextKind::Refusal,
			other => return Err(DecodeError::UnsupportedItem(other.to_string())),
		};
		self.open_text_part(wire.output_index, wire.content_index, kind, sink)?;
		Ok(())
	}

	/// Some providers send no `content_part.added`, so any event for a new part
	/// may start it.
	fn open_text_part(
		&mut self,
		output_index: Option<u32>,
		content_index: Option<u32>,
		kind: TextKind,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(usize, usize), DecodeError> {
		let content_index = content_index.unwrap_or(0);
		let item_at = self.message_index(output_index)?;
		let ItemState::Message { parts } = &self.items[item_at].state else {
			return Err(DecodeError::OutOfOrder(
				"text event for an item that is not a message",
			));
		};
		if let Some(at) = parts
			.iter()
			.position(|part| part.content_index == content_index)
		{
			if parts[at].kind != kind {
				return Err(DecodeError::OutOfOrder("text and refusal in one part"));
			}
			return Ok((item_at, at));
		}
		let id = self.parts.allocate()?;
		sink(Event::PartStart {
			part: id,
			kind: match kind {
				TextKind::Text => PartKind::Text,
				TextKind::Refusal => PartKind::Refusal,
			},
		});
		let ItemState::Message { parts } = &mut self.items[item_at].state else {
			return Err(DecodeError::OutOfOrder(
				"text event for an item that is not a message",
			));
		};
		parts.push(TextPart {
			content_index,
			id,
			kind,
			streamed: Check::EMPTY,
			open: true,
		});
		Ok((item_at, parts.len() - 1))
	}

	fn message_index(&mut self, output_index: Option<u32>) -> Result<usize, DecodeError> {
		self.open_item(output_index)?;
		self
			.items
			.iter()
			.position(|item| Some(item.output_index) == output_index)
			.ok_or(DecodeError::OutOfOrder(
				"event for an item that is not open",
			))
	}

	fn open_item(&mut self, output_index: Option<u32>) -> Result<&mut Item, DecodeError> {
		self.require_started()?;
		let output_index = output_index.ok_or(DecodeError::WrongType("output_index"))?;
		self
			.items
			.iter_mut()
			.find(|item| item.output_index == output_index && item.open)
			.ok_or(DecodeError::OutOfOrder(
				"event for an item that is not open",
			))
	}

	fn text_delta(
		&mut self,
		wire: &Wire<'_>,
		kind: TextKind,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let text = string(wire.delta, "delta")?;
		let (item, at) = self.open_text_part(wire.output_index, wire.content_index, kind, sink)?;
		let ItemState::Message { parts } = &mut self.items[item].state else {
			return Err(DecodeError::OutOfOrder(
				"text event for an item that is not a message",
			));
		};
		let part = &mut parts[at];
		if !part.open {
			return Err(DecodeError::OutOfOrder("delta after its part ended"));
		}
		part.streamed.add(&text.decode());
		sink(Event::Delta {
			part: part.id,
			text,
		});
		Ok(())
	}

	/// Only URL citations have a Messages form. Others are ignored on purpose.
	fn annotation(
		&mut self,
		wire: &Wire<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let annotation = wire
			.annotation
			.as_ref()
			.ok_or(DecodeError::WrongType("annotation"))?;
		if annotation.kind != "url_citation" {
			return self.require_started();
		}
		let url = annotation
			.url
			.as_deref()
			.ok_or(DecodeError::WrongType("url"))?;
		let (item, at) =
			self.open_text_part(wire.output_index, wire.content_index, TextKind::Text, sink)?;
		let ItemState::Message { parts } = &self.items[item].state else {
			return Err(DecodeError::OutOfOrder(
				"text event for an item that is not a message",
			));
		};
		if !parts[at].open {
			return Err(DecodeError::OutOfOrder("annotation after its part ended"));
		}
		sink(Event::Citation {
			part: parts[at].id,
			url,
			title: annotation.title.as_deref(),
		});
		Ok(())
	}

	fn text_done(
		&mut self,
		output_index: Option<u32>,
		content_index: Option<u32>,
		kind: TextKind,
		text: Option<&RawValue>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let text = string(text, "text")?;
		let (item, at) = self.open_text_part(output_index, content_index, kind, sink)?;
		let ItemState::Message { parts } = &mut self.items[item].state else {
			return Err(DecodeError::OutOfOrder(
				"text event for an item that is not a message",
			));
		};
		let part = &mut parts[at];
		if !part.open {
			return Err(DecodeError::OutOfOrder("repeated text completion"));
		}
		close_text(part, text, sink)
	}

	fn arguments_delta(
		&mut self,
		wire: &Wire<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let text = string(wire.delta, "delta")?;
		let item = self.open_item(wire.output_index)?;
		let ItemState::ToolCall {
			id,
			streamed,
			arguments_done,
		} = &mut item.state
		else {
			return Err(DecodeError::OutOfOrder(
				"arguments for an item that is not a call",
			));
		};
		if *arguments_done {
			return Err(DecodeError::OutOfOrder("delta after arguments ended"));
		}
		streamed.add(&text.decode());
		sink(Event::Delta { part: *id, text });
		Ok(())
	}

	fn arguments_done(&mut self, wire: &Wire<'_>) -> Result<(), DecodeError> {
		let text = string(wire.arguments, "arguments")?;
		let item = self.open_item(wire.output_index)?;
		let ItemState::ToolCall {
			streamed,
			arguments_done,
			..
		} = &mut item.state
		else {
			return Err(DecodeError::OutOfOrder(
				"arguments for an item that is not a call",
			));
		};
		if *arguments_done {
			return Err(DecodeError::OutOfOrder("repeated arguments completion"));
		}
		*arguments_done = true;
		if *streamed != Check::of(&text.decode()) {
			return Err(DecodeError::FinalDiffers("arguments"));
		}
		Ok(())
	}

	fn reasoning_delta(
		&mut self,
		wire: &Wire<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let text = string(wire.delta, "delta")?;
		let item = self.open_item(wire.output_index)?;
		let ItemState::Reasoning {
			id,
			streamed_summary,
			..
		} = &mut item.state
		else {
			return Err(DecodeError::OutOfOrder(
				"reasoning for an item that is not reasoning",
			));
		};
		// Raw reasoning text and summary text are different streams.
		if wire.kind == "response.reasoning_summary_text.delta" {
			streamed_summary
				.get_or_insert(Check::EMPTY)
				.add(&text.decode());
		}
		sink(Event::Delta { part: *id, text });
		Ok(())
	}

	fn item_done(
		&mut self,
		wire: &Wire<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let raw = wire.item.ok_or(DecodeError::WrongType("item"))?;
		let parsed: WireItem<'_> = serde_json::from_str(raw.get())?;
		let item_at = self.message_index(wire.output_index)?;
		self.items[item_at].open = false;
		match &self.items[item_at].state {
			ItemState::Message { .. } => {
				self.message_done(item_at, wire.output_index, &parsed, raw, sink)
			},
			ItemState::ToolCall { .. } => self.call_done(item_at, &parsed, sink),
			ItemState::Reasoning { .. } => self.reasoning_done(item_at, &parsed, raw, sink),
		}
	}

	fn message_done(
		&mut self,
		item_at: usize,
		output_index: Option<u32>,
		parsed: &WireItem<'_>,
		raw: &RawValue,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if parsed.kind != "message" {
			return Err(DecodeError::OutOfOrder("item changed type"));
		}
		self.items[item_at].open = true;
		for (index, content) in parsed.content.iter().enumerate() {
			let (kind, text) = match content.kind.as_ref() {
				"output_text" => (TextKind::Text, string(content.text, "text")?),
				"refusal" => (TextKind::Refusal, string(content.refusal, "refusal")?),
				other => return Err(DecodeError::UnsupportedItem(other.to_string())),
			};
			let content_index = Some(u32::try_from(index).map_err(|_| DecodeError::TooManyParts)?);
			let (item, at) = self.open_text_part(output_index, content_index, kind, sink)?;
			let ItemState::Message { parts } = &mut self.items[item].state else {
				return Err(DecodeError::OutOfOrder(
					"text event for an item that is not a message",
				));
			};
			let part = &mut parts[at];
			if part.open {
				close_text(part, text, sink)?;
			} else if part.streamed != Check::of(&text.decode()) {
				return Err(DecodeError::FinalDiffers("text"));
			}
		}
		let item = &mut self.items[item_at];
		item.open = false;
		let ItemState::Message { parts } = &item.state else {
			return Err(DecodeError::OutOfOrder(
				"text event for an item that is not a message",
			));
		};
		if parts.iter().any(|part| part.open) {
			return Err(DecodeError::OutOfOrder(
				"message ended with a part still open",
			));
		}
		sink(Event::MessageEnd { item: raw });
		Ok(())
	}

	fn call_done(
		&self,
		item_at: usize,
		parsed: &WireItem<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let ItemState::ToolCall { id, streamed, .. } = &self.items[item_at].state else {
			return Err(DecodeError::OutOfOrder("item changed type"));
		};
		if parsed.kind != "function_call" {
			return Err(DecodeError::OutOfOrder("item changed type"));
		}
		let arguments = string(parsed.arguments, "arguments")?;
		let decoded = arguments.decode();
		if *streamed != Check::of(&decoded) {
			return Err(DecodeError::FinalDiffers("arguments"));
		}
		let arguments = if parsed.status.as_deref() == Some("incomplete") {
			Arguments::Partial(arguments)
		} else {
			Arguments::complete(arguments).ok_or(DecodeError::InvalidArguments)?
		};
		sink(Event::PartEnd {
			part: *id,
			value: Final::ToolCall(arguments),
		});
		Ok(())
	}

	fn reasoning_done(
		&mut self,
		item_at: usize,
		parsed: &WireItem<'_>,
		raw: &RawValue,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let ItemState::Reasoning {
			id,
			encrypted,
			streamed_summary,
		} = &mut self.items[item_at].state
		else {
			return Err(DecodeError::OutOfOrder("item changed type"));
		};
		if parsed.kind != "reasoning" {
			return Err(DecodeError::OutOfOrder("item changed type"));
		}
		let value = parsed
			.encrypted_content
			.map(|raw| JsonStr::read(raw, "encrypted_content"))
			.transpose()?;
		*encrypted = value.is_some();
		let text = summary_text(raw)?;
		let summary = JsonStr::read(&text, "summary")?;
		if streamed_summary.is_some_and(|streamed| streamed != Check::of(&summary.decode())) {
			return Err(DecodeError::FinalDiffers("reasoning summary"));
		}
		sink(Event::PartEnd {
			part: *id,
			value: Final::Reasoning {
				text: Some(summary),
				encrypted: value,
				item: raw,
			},
		});
		Ok(())
	}

	fn terminal(
		&mut self,
		wire: &Wire<'_>,
		kind: Terminal,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		self.require_started()?;
		let raw = wire.response.ok_or(DecodeError::WrongType("response"))?;
		let terminal: WireTerminal<'_> = serde_json::from_str(raw.get())?;
		let output: Vec<TerminalItem> = terminal
			.output
			.iter()
			.map(|item| TerminalItem {
				reasoning: item.kind == "reasoning",
				has_state: item
					.encrypted_content
					.is_some_and(|value| value.get() != "null"),
			})
			.collect();
		let details = terminal.incomplete_details.as_ref();
		let error = terminal.error.as_ref();
		let record = TerminalRecord {
			output: &output,
			service_tier: terminal.service_tier.as_deref(),
			usage: terminal.usage.map(Usage::from),
			reason: details.and_then(|details| details.reason.as_deref()),
			code: error.and_then(|error| error.code.as_deref()),
			message: error.and_then(|error| error.message.as_deref()),
		};
		self.end_response(kind, &record, sink)
	}

	fn end_response(
		&mut self,
		kind: Terminal,
		record: &TerminalRecord<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		// Usage goes first: tokens were spent even if the checks below fail.
		record.send_usage(sink);
		if kind != Terminal::Failed {
			self.check_terminal(record.output)?;
		}
		self.stage = Stage::Finished;
		record.send_end(kind, sink);
		Ok(())
	}

	fn check_terminal(&self, output: &[TerminalItem]) -> Result<(), DecodeError> {
		if self.items.iter().any(|item| item.open) {
			return Err(DecodeError::OutOfOrder(
				"terminal event with an item still open",
			));
		}
		if output.is_empty() {
			return Ok(());
		}
		if output.len() != self.items.len() {
			return Err(DecodeError::LateOutput("output"));
		}
		// A different value here is fine: completion can re-encrypt reasoning,
		// and the value from the item's completion is the one to replay.
		for (item, seen) in output.iter().zip(&self.items) {
			let late = item.reasoning
				&& item.has_state
				&& matches!(
					seen.state,
					ItemState::Reasoning {
						encrypted: false,
						..
					}
				);
			if late {
				return Err(DecodeError::LateOutput("reasoning state"));
			}
		}
		Ok(())
	}
}

const WORD: usize = size_of::<u64>();

/// What has been streamed for a part, without keeping it. Hashes a word at a time,
/// because a closing event repeats the whole reply. `tail` keeps the bytes of a word
/// not yet complete, so the result doesn't depend on how the text was split.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Check {
	length: u64,
	hash: u64,
	tail: [u8; WORD],
}

impl Check {
	const EMPTY: Self = Self {
		length: 0,
		hash: 0xcbf2_9ce4_8422_2325,
		tail: [0; WORD],
	};

	fn add(&mut self, text: &str) {
		let mut bytes = text.as_bytes();
		let used = usize::try_from(self.length % WORD as u64)
			.unwrap_or_else(|_| unreachable!("a partial word fits in usize"));
		self.length += bytes.len() as u64;
		if used > 0 {
			let take = (WORD - used).min(bytes.len());
			let (head, rest) = bytes.split_at(take);
			self.tail[used..used + take].copy_from_slice(head);
			bytes = rest;
			if used + take < WORD {
				return;
			}
			self.mix(self.tail);
			self.tail = [0; WORD];
		}
		let (words, rest) = bytes.as_chunks::<WORD>();
		for word in words {
			self.mix(*word);
		}
		self.tail[..rest.len()].copy_from_slice(rest);
	}

	const fn mix(&mut self, word: [u8; WORD]) {
		self.hash = (self.hash ^ u64::from_le_bytes(word)).wrapping_mul(0x0100_0000_01b3);
	}

	fn of(text: &str) -> Self {
		let mut check = Self::EMPTY;
		check.add(text);
		check
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextKind {
	Text,
	Refusal,
}

struct TextPart {
	content_index: u32,
	id: PartId,
	kind: TextKind,
	streamed: Check,
	open: bool,
}

enum ItemState {
	Message {
		parts: Vec<TextPart>,
	},
	ToolCall {
		id: PartId,
		streamed: Check,
		arguments_done: bool,
	},
	Reasoning {
		id: PartId,
		encrypted: bool,
		streamed_summary: Option<Check>,
	},
}

struct Item {
	output_index: u32,
	open: bool,
	state: ItemState,
}

fn close_text(
	part: &mut TextPart,
	text: JsonStr<'_>,
	sink: &mut dyn FnMut(Event<'_>),
) -> Result<(), DecodeError> {
	let check = Check::of(&text.decode());
	// A refusal may arrive whole. Text with no deltas means deltas were lost.
	let arrived_whole = part.kind == TextKind::Refusal && part.streamed == Check::EMPTY;
	if part.streamed != check && !arrived_whole {
		return Err(DecodeError::FinalDiffers("text"));
	}
	part.streamed = check;
	part.open = false;
	sink(Event::PartEnd {
		part: part.id,
		value: match part.kind {
			TextKind::Text => Final::Text,
			TextKind::Refusal => Final::Refusal(text),
		},
	});
	Ok(())
}

/// Gives the same events as a stream.
pub fn decode_buffered(body: &[u8], sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
	let response: WireBuffered<'_> = serde_json::from_slice(body)?;
	sink(Event::ResponseStart {
		id: &response.id,
		model: &response.model,
	});
	let record = TerminalRecord {
		output: &[],
		service_tier: response.service_tier.as_deref(),
		usage: response.usage.map(Usage::from),
		reason: response
			.incomplete_details
			.as_ref()
			.and_then(|details| details.reason.as_deref()),
		code: response
			.error
			.as_ref()
			.and_then(|error| error.code.as_deref()),
		message: response
			.error
			.as_ref()
			.and_then(|error| error.message.as_deref()),
	};
	record.send_usage(sink);
	if response.output.len() > MAX_PARTS {
		return Err(DecodeError::TooManyParts);
	}
	let mut parts = PartIds::default();
	for raw in &response.output {
		let item: WireItem<'_> = serde_json::from_str(raw.get())?;
		match item.kind.as_ref() {
			"message" => {
				for content in &item.content {
					let (kind, delta, value) = match content.kind.as_ref() {
						"output_text" => {
							let text = string(content.text, "text")?;
							(PartKind::Text, Some(text), Final::Text)
						},
						"refusal" => {
							let text = string(content.refusal, "refusal")?;
							(PartKind::Refusal, None, Final::Refusal(text))
						},
						other => return Err(DecodeError::UnsupportedItem(other.to_string())),
					};
					let part = parts.allocate()?;
					sink(Event::PartStart { part, kind });
					for annotation in &content.annotations {
						if let ("url_citation", Some(url)) = (annotation.kind.as_ref(), &annotation.url) {
							sink(Event::Citation {
								part,
								url,
								title: annotation.title.as_deref(),
							});
						}
					}
					// A final text value has no text, so it goes as one delta.
					if let Some(text) = delta.filter(|text| text.raw() != r#""""#) {
						sink(Event::Delta { part, text });
					}
					sink(Event::PartEnd { part, value });
				}
				sink(Event::MessageEnd { item: raw });
			},
			"function_call" => {
				let call_id = item.call_id.ok_or(DecodeError::WrongType("call_id"))?;
				let name = item.name.ok_or(DecodeError::WrongType("name"))?;
				let arguments = string(item.arguments, "arguments")?;
				let arguments = if item.status.as_deref() == Some("incomplete") {
					Arguments::Partial(arguments)
				} else {
					Arguments::complete(arguments).ok_or(DecodeError::InvalidArguments)?
				};
				let part = parts.allocate()?;
				let kind = PartKind::ToolCall {
					call_id: &call_id,
					name: &name,
				};
				sink(Event::PartStart { part, kind });
				let value = Final::ToolCall(arguments);
				sink(Event::PartEnd { part, value });
			},
			"reasoning" => {
				let part = parts.allocate()?;
				sink(Event::PartStart {
					part,
					kind: PartKind::Reasoning,
				});
				let text = summary_text(raw)?;
				let value = Final::Reasoning {
					text: JsonStr::new(&text),
					encrypted: item
						.encrypted_content
						.map(|raw| JsonStr::read(raw, "encrypted_content"))
						.transpose()?,
					item: raw,
				};
				sink(Event::PartEnd { part, value });
			},
			other => return Err(DecodeError::UnsupportedItem(other.to_string())),
		}
	}
	let kind = Terminal::from_status(&response.status).ok_or(DecodeError::WrongType("status"))?;
	record.send_end(kind, sink);
	Ok(())
}

#[derive(Deserialize)]
struct WireBuffered<'a> {
	#[serde(borrow)]
	id: Cow<'a, str>,
	#[serde(borrow)]
	model: Cow<'a, str>,
	#[serde(borrow)]
	status: Cow<'a, str>,
	#[serde(default, borrow)]
	output: Vec<&'a RawValue>,
	#[serde(borrow)]
	service_tier: Option<Cow<'a, str>>,
	usage: Option<WireUsage>,
	#[serde(borrow)]
	incomplete_details: Option<WireIncomplete<'a>>,
	#[serde(borrow)]
	error: Option<WireError<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Terminal {
	Completed,
	Incomplete,
	Failed,
}

impl Terminal {
	pub(super) fn from_event(kind: &str) -> Option<Self> {
		match kind {
			"response.completed" => Some(Self::Completed),
			"response.incomplete" => Some(Self::Incomplete),
			"response.failed" => Some(Self::Failed),
			_ => None,
		}
	}

	pub(super) fn from_status(status: &str) -> Option<Self> {
		match status {
			"completed" => Some(Self::Completed),
			"incomplete" => Some(Self::Incomplete),
			"failed" => Some(Self::Failed),
			_ => None,
		}
	}
}

pub(super) struct TerminalItem {
	pub reasoning: bool,
	pub has_state: bool,
}

pub(super) struct TerminalRecord<'a> {
	pub output: &'a [TerminalItem],
	pub service_tier: Option<&'a str>,
	pub usage: Option<Usage>,
	pub reason: Option<&'a str>,
	pub code: Option<&'a str>,
	pub message: Option<&'a str>,
}

impl TerminalRecord<'_> {
	pub(super) fn send_usage(&self, sink: &mut dyn FnMut(Event<'_>)) {
		if let Some(tier) = self.service_tier {
			sink(Event::ServiceTier(tier));
		}
		if let Some(usage) = self.usage {
			sink(Event::Usage(usage));
		}
	}

	pub(super) fn send_end(&self, kind: Terminal, sink: &mut dyn FnMut(Event<'_>)) {
		match kind {
			Terminal::Completed => sink(Event::Finish(Outcome::Completed)),
			Terminal::Incomplete => sink(Event::Finish(incomplete(self.reason))),
			Terminal::Failed => sink(Event::Error(provider_failure(self.code, self.message))),
		}
	}
}

/// Any body is accepted, JSON or not.
#[must_use]
pub fn http_error_kind(body: &[u8]) -> ErrorKind {
	#[derive(Deserialize)]
	struct Body<'a> {
		#[serde(borrow)]
		error: Option<Detail<'a>>,
	}
	#[derive(Deserialize)]
	struct Detail<'a> {
		#[serde(borrow, default)]
		code: Option<Cow<'a, str>>,
		#[serde(rename = "type", borrow, default)]
		kind: Option<Cow<'a, str>>,
	}
	let parsed: Option<Body<'_>> = serde_json::from_slice(body).ok();
	let detail = parsed.and_then(|parsed| parsed.error);
	error_kind(
		detail
			.as_ref()
			.and_then(|detail| detail.code.as_deref().or(detail.kind.as_deref())),
	)
}

#[must_use]
pub fn http_error_field(body: &[u8]) -> Option<String> {
	openai_error_field(body)
}

fn incomplete(raw: Option<&str>) -> Outcome<'_> {
	let reason = match raw {
		Some("max_output_tokens") => StopReason::MaxTokens,
		Some("content_filter") => StopReason::ContentFilter,
		Some("max_context_length") => StopReason::ContextWindow,
		_ => StopReason::Other,
	};
	Outcome::Incomplete { reason, raw }
}

pub(super) fn provider_failure<'a>(
	code: Option<&'a str>,
	message: Option<&'a str>,
) -> ProviderError<'a> {
	ProviderError {
		kind: error_kind(code),
		code,
		message,
		client_message: None,
	}
}

fn error_kind(code: Option<&str>) -> ErrorKind {
	match code {
		Some("context_length_exceeded") => ErrorKind::ContextTooLong,
		Some("insufficient_quota" | "credit_balance_exhausted") => ErrorKind::Billing,
		Some("rate_limit_exceeded") => ErrorKind::RateLimit,
		Some("server_is_overloaded") => ErrorKind::Overloaded,
		Some("invalid_prompt") => ErrorKind::InvalidRequest,
		_ => ErrorKind::Unknown,
	}
}

#[cfg(test)]
#[path = "decoder_tests.rs"]
mod tests;
