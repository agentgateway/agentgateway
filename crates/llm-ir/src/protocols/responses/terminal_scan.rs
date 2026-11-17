//! Reads oversized Responses terminal events without retaining repeated output text.
//!
//! All bytes are checked as JSON. Retained values, nesting depth, and output item
//! count have separate limits; a larger repeated text value needs no extra storage.

use std::borrow::Cow;

use super::decoder::{Terminal, TerminalItem, TerminalRecord};
use super::wire::{WireError, WireIncomplete, WireUsage};
use crate::ir::events::{DecodeError, Usage};

const MAX_DEPTH: usize = 128;
/// Usage and outcome values are small. More than this is an error.
const MAX_KEPT_VALUE: usize = 1 << 16;
/// Longer than any key looked for here.
const MAX_KEY: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
	Root,
	Response,
	Output,
	Item,
	Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
	Root,
	Kind,
	Response,
	ServiceTier,
	Usage,
	Incomplete,
	Error,
	Output,
	Item,
	ItemKind,
	ItemState,
	Skip,
}

#[derive(Clone, Copy)]
enum State {
	Value,
	ValueOrEnd,
	KeyOrEnd,
	Key,
	InString { key: bool },
	Escape { key: bool },
	Hex { key: bool, left: u8 },
	Colon,
	AfterValue,
	Minus,
	Zero,
	Int,
	Dot,
	Frac,
	Exp,
	ExpSign,
	ExpInt,
	Word { word: &'static [u8], at: usize },
	Done,
}

#[derive(Default)]
struct Seen {
	kind: bool,
	response: bool,
	service_tier: bool,
	usage: bool,
	incomplete: bool,
	error: bool,
	output: bool,
}

#[derive(Default)]
struct ItemSeen {
	kind: bool,
	state: bool,
	reasoning: bool,
	has_state: bool,
}

#[derive(Default)]
pub struct Scanned {
	pub kind: Option<Terminal>,
	pub service_tier: Option<String>,
	pub usage: Option<Usage>,
	pub reason: Option<String>,
	pub code: Option<String>,
	pub message: Option<String>,
	pub output: Vec<TerminalItem>,
}

impl Scanned {
	pub fn record(&self) -> TerminalRecord<'_> {
		TerminalRecord {
			output: &self.output,
			service_tier: self.service_tier.as_deref(),
			usage: self.usage,
			reason: self.reason.as_deref(),
			code: self.code.as_deref(),
			message: self.message.as_deref(),
		}
	}
}

struct KeptValue {
	slot: Slot,
	start_depth: usize,
}

pub struct TerminalScanner {
	max_event_bytes: usize,
	state: State,
	stack: Vec<Place>,
	arrays: Vec<bool>,
	key: Vec<u8>,
	key_too_long: bool,
	utf8_tail: [u8; 4],
	utf8_len: usize,
	kept: Vec<u8>,
	keeping: Option<KeptValue>,
	value_ended: bool,
	seen: Seen,
	item: ItemSeen,
	scanned: Scanned,
}

impl TerminalScanner {
	pub fn new(max_event_bytes: usize) -> Self {
		Self {
			max_event_bytes,
			state: State::Value,
			stack: Vec::new(),
			arrays: Vec::new(),
			key: Vec::new(),
			key_too_long: false,
			utf8_tail: [0; 4],
			utf8_len: 0,
			kept: Vec::new(),
			keeping: None,
			value_ended: false,
			seen: Seen::default(),
			item: ItemSeen::default(),
			scanned: Scanned::default(),
		}
	}

	pub fn push(&mut self, piece: &[u8]) -> Result<(), DecodeError> {
		self.check_utf8(piece)?;
		let mut at = 0;
		while let Some(byte) = piece.get(at) {
			let key_byte = match self.state {
				State::InString { key: true } => *byte != b'"',
				State::Escape { key: true } | State::Hex { key: true, .. } => true,
				_ => false,
			};
			if key_byte && self.stack.last() != Some(&Place::Other) {
				// Even an all-escaped spelling of a recognized key fits here.
				if self.key.len() == MAX_KEY * 6 {
					self.key_too_long = true;
				} else {
					self.key.push(*byte);
				}
			}
			let consumed = self.step(*byte)?;
			if consumed {
				at += 1;
				if self.keeping.is_some() {
					if self.kept.len() == MAX_KEPT_VALUE {
						return Err(DecodeError::WrongType("value too large in terminal event"));
					}
					self.kept.push(*byte);
				}
			}
			if std::mem::take(&mut self.value_ended) {
				self.finish_kept_value()?;
			}
		}
		Ok(())
	}

	pub fn finish(self) -> Result<Scanned, DecodeError> {
		std::str::from_utf8(&self.utf8_tail[..self.utf8_len])?;
		if !matches!(self.state, State::Done) {
			return Err(malformed("JSON ended early"));
		}
		if self.scanned.kind.is_none() {
			return Err(DecodeError::WrongType("type"));
		}
		if !self.seen.response {
			return Err(DecodeError::WrongType("response"));
		}
		Ok(self.scanned)
	}

	fn check_utf8(&mut self, mut piece: &[u8]) -> Result<(), DecodeError> {
		while self.utf8_len > 0 {
			let Some((byte, rest)) = piece.split_first() else {
				return Ok(());
			};
			self.utf8_tail[self.utf8_len] = *byte;
			self.utf8_len += 1;
			piece = rest;
			match std::str::from_utf8(&self.utf8_tail[..self.utf8_len]) {
				Ok(_) => self.utf8_len = 0,
				Err(error) if error.error_len().is_some() => return Err(error.into()),
				Err(_) => {},
			}
		}
		if let Err(error) = std::str::from_utf8(piece) {
			if error.error_len().is_some() {
				return Err(error.into());
			}
			let tail = &piece[error.valid_up_to()..];
			self.utf8_tail[..tail.len()].copy_from_slice(tail);
			self.utf8_len = tail.len();
		}
		Ok(())
	}

	fn step(&mut self, byte: u8) -> Result<bool, DecodeError> {
		match self.state {
			State::Value
			| State::ValueOrEnd
			| State::KeyOrEnd
			| State::Key
			| State::Colon
			| State::AfterValue
			| State::Done
				if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') =>
			{
				Ok(true)
			},
			State::Value => self.begin_value(byte),
			State::ValueOrEnd => {
				if byte == b']' {
					self.close()?;
					Ok(true)
				} else {
					self.begin_value(byte)
				}
			},
			State::KeyOrEnd if byte == b'}' => {
				self.close()?;
				Ok(true)
			},
			State::KeyOrEnd | State::Key => {
				if byte != b'"' {
					return Err(malformed("expected a key"));
				}
				self.key.clear();
				self.key_too_long = false;
				self.state = State::InString { key: true };
				Ok(true)
			},
			State::InString { key } => self.in_string(byte, key),
			State::Escape { key } => {
				self.state = match byte {
					b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => State::InString { key },
					b'u' => State::Hex { key, left: 4 },
					_ => return Err(malformed("invalid escape")),
				};
				Ok(true)
			},
			State::Hex { key, left } => {
				if !byte.is_ascii_hexdigit() {
					return Err(malformed("invalid escape"));
				}
				self.state = if left == 1 {
					State::InString { key }
				} else {
					State::Hex {
						key,
						left: left - 1,
					}
				};
				Ok(true)
			},
			State::Colon => {
				if byte != b':' {
					return Err(malformed("expected a colon"));
				}
				self.state = State::Value;
				Ok(true)
			},
			State::AfterValue => self.after_value(byte),
			State::Minus => match byte {
				b'0' => self.to(State::Zero),
				b'1'..=b'9' => self.to(State::Int),
				_ => Err(malformed("invalid number")),
			},
			State::Zero => match byte {
				b'.' => self.to(State::Dot),
				b'e' | b'E' => self.to(State::Exp),
				_ => self.end_scalar(false),
			},
			State::Int => match byte {
				b'0'..=b'9' => Ok(true),
				b'.' => self.to(State::Dot),
				b'e' | b'E' => self.to(State::Exp),
				_ => self.end_scalar(false),
			},
			State::Dot => match byte {
				b'0'..=b'9' => self.to(State::Frac),
				_ => Err(malformed("invalid number")),
			},
			State::Frac => match byte {
				b'0'..=b'9' => Ok(true),
				b'e' | b'E' => self.to(State::Exp),
				_ => self.end_scalar(false),
			},
			State::Exp => match byte {
				b'+' | b'-' => self.to(State::ExpSign),
				b'0'..=b'9' => self.to(State::ExpInt),
				_ => Err(malformed("invalid number")),
			},
			State::ExpSign => match byte {
				b'0'..=b'9' => self.to(State::ExpInt),
				_ => Err(malformed("invalid number")),
			},
			State::ExpInt => match byte {
				b'0'..=b'9' => Ok(true),
				_ => self.end_scalar(false),
			},
			State::Word { word, at } => {
				if word.get(at) != Some(&byte) {
					return Err(malformed("invalid word"));
				}
				if at + 1 == word.len() {
					self.end_scalar(true)
				} else {
					self.to(State::Word { word, at: at + 1 })
				}
			},
			State::Done => Err(malformed("bytes after the value")),
		}
	}

	const fn to(&mut self, state: State) -> Result<bool, DecodeError> {
		self.state = state;
		Ok(true)
	}

	fn in_string(&mut self, byte: u8, key: bool) -> Result<bool, DecodeError> {
		let tracked = key
			&& self
				.stack
				.last()
				.is_some_and(|place| *place != Place::Other);
		match byte {
			b'"' => {
				if key {
					if tracked && !self.key_too_long && self.key.contains(&b'\\') {
						self.key.insert(0, b'"');
						self.key.push(b'"');
						let decoded: String = serde_json::from_slice(&self.key)?;
						self.key.clear();
						self.key.extend_from_slice(decoded.as_bytes());
					}
					self.state = State::Colon;
					Ok(true)
				} else {
					self.end_scalar(true)
				}
			},
			b'\\' => self.to(State::Escape { key }),
			0..0x20 => Err(malformed("control character in a string")),
			_ => Ok(true),
		}
	}

	fn slot(&self) -> Slot {
		let Some(place) = self.stack.last() else {
			return Slot::Root;
		};
		if *place == Place::Output {
			return Slot::Item;
		}
		if self.key_too_long {
			return Slot::Skip;
		}
		match (place, self.key.as_slice()) {
			(Place::Root, b"type") => Slot::Kind,
			(Place::Root, b"response") => Slot::Response,
			(Place::Response, b"service_tier") => Slot::ServiceTier,
			(Place::Response, b"usage") => Slot::Usage,
			(Place::Response, b"incomplete_details") => Slot::Incomplete,
			(Place::Response, b"error") => Slot::Error,
			(Place::Response, b"output") => Slot::Output,
			(Place::Item, b"type") => Slot::ItemKind,
			(Place::Item, b"encrypted_content") => Slot::ItemState,
			_ => Slot::Skip,
		}
	}

	fn begin_value(&mut self, byte: u8) -> Result<bool, DecodeError> {
		let slot = self.slot();
		self.note_slot(slot)?;
		match (slot, byte) {
			(Slot::Root, b) if b != b'{' => return Err(DecodeError::WrongType("terminal event")),
			(Slot::Response, b) if b != b'{' => return Err(DecodeError::WrongType("response")),
			(Slot::Output, b) if b != b'[' => return Err(DecodeError::WrongType("output")),
			(Slot::Item, b) if b != b'{' => return Err(DecodeError::WrongType("item")),
			_ => {},
		}
		if matches!(
			slot,
			Slot::Kind
				| Slot::ServiceTier
				| Slot::Usage
				| Slot::Incomplete
				| Slot::Error
				| Slot::ItemKind
		) {
			self.kept.clear();
			self.keeping = Some(KeptValue {
				slot,
				start_depth: self.stack.len(),
			});
		}
		if slot == Slot::ItemState {
			self.item.has_state = byte != b'n';
		}
		let place = match slot {
			Slot::Root => Place::Root,
			Slot::Response => Place::Response,
			Slot::Output => Place::Output,
			Slot::Item => Place::Item,
			_ => Place::Other,
		};
		match byte {
			b'{' => {
				self.open(place, false)?;
				self.to(State::KeyOrEnd)
			},
			b'[' => {
				self.open(place, true)?;
				self.to(State::ValueOrEnd)
			},
			b'"' => self.to(State::InString { key: false }),
			b'-' => self.to(State::Minus),
			b'0' => self.to(State::Zero),
			b'1'..=b'9' => self.to(State::Int),
			b't' => self.to(State::Word {
				word: b"true",
				at: 1,
			}),
			b'f' => self.to(State::Word {
				word: b"false",
				at: 1,
			}),
			b'n' => self.to(State::Word {
				word: b"null",
				at: 1,
			}),
			_ => Err(malformed("expected a value")),
		}
	}

	const fn note_slot(&mut self, slot: Slot) -> Result<(), DecodeError> {
		let seen = match slot {
			Slot::Kind => &mut self.seen.kind,
			Slot::Response => &mut self.seen.response,
			Slot::ServiceTier => &mut self.seen.service_tier,
			Slot::Usage => &mut self.seen.usage,
			Slot::Incomplete => &mut self.seen.incomplete,
			Slot::Error => &mut self.seen.error,
			Slot::Output => &mut self.seen.output,
			Slot::ItemKind => &mut self.item.kind,
			Slot::ItemState => &mut self.item.state,
			Slot::Root | Slot::Item | Slot::Skip => return Ok(()),
		};
		if std::mem::replace(seen, true) {
			return Err(malformed("repeated field"));
		}
		Ok(())
	}

	fn open(&mut self, place: Place, array: bool) -> Result<(), DecodeError> {
		if self.stack.len() == MAX_DEPTH {
			return Err(malformed("nested too deeply"));
		}
		if place == Place::Item {
			self.item = ItemSeen::default();
		}
		self.stack.push(place);
		self.arrays.push(array);
		Ok(())
	}

	fn after_value(&mut self, byte: u8) -> Result<bool, DecodeError> {
		let in_array = self.arrays.last().copied();
		match (in_array, byte) {
			(Some(true), b',') => self.to(State::Value),
			(Some(false), b',') => self.to(State::Key),
			(Some(true), b']') | (Some(false), b'}') => {
				self.close()?;
				Ok(true)
			},
			_ => Err(malformed("expected a comma or an end")),
		}
	}

	fn close(&mut self) -> Result<(), DecodeError> {
		self.arrays.pop();
		if self.stack.pop() == Some(Place::Item) {
			if !self.item.kind {
				return Err(DecodeError::WrongType("item type"));
			}
			if self.scanned.output.len() == crate::ir::events::MAX_PARTS {
				return Err(DecodeError::TooManyParts);
			}
			self.scanned.output.push(TerminalItem {
				reasoning: self.item.reasoning,
				has_state: self.item.has_state,
			});
		}
		self.end_value();
		Ok(())
	}

	fn end_scalar(&mut self, consumed: bool) -> Result<bool, DecodeError> {
		self.end_value();
		Ok(consumed)
	}

	fn end_value(&mut self) {
		self.state = if self.stack.is_empty() {
			State::Done
		} else {
			State::AfterValue
		};
		self.value_ended = self
			.keeping
			.as_ref()
			.is_some_and(|kept| kept.start_depth == self.stack.len());
	}

	fn finish_kept_value(&mut self) -> Result<(), DecodeError> {
		let Some(KeptValue { slot, .. }) = self.keeping.take() else {
			return Ok(());
		};
		match slot {
			Slot::Kind => {
				let kind: Cow<'_, str> = serde_json::from_slice(&self.kept)?;
				let kind = Terminal::from_event(&kind).ok_or(DecodeError::EventTooLarge {
					limit: self.max_event_bytes,
				})?;
				self.scanned.kind = Some(kind);
			},
			Slot::ServiceTier => {
				let tier: Option<Cow<'_, str>> = serde_json::from_slice(&self.kept)?;
				self.scanned.service_tier = tier.map(Cow::into_owned);
			},
			Slot::Usage => {
				let usage: Option<WireUsage> = serde_json::from_slice(&self.kept)?;
				self.scanned.usage = usage.map(Usage::from);
			},
			Slot::Incomplete => {
				let details: Option<WireIncomplete<'_>> = serde_json::from_slice(&self.kept)?;
				self.scanned.reason = details.and_then(|details| details.reason.map(Cow::into_owned));
			},
			Slot::Error => {
				let error: Option<WireError<'_>> = serde_json::from_slice(&self.kept)?;
				if let Some(error) = error {
					self.scanned.code = error.code.map(Cow::into_owned);
					self.scanned.message = error.message.map(Cow::into_owned);
				}
			},
			Slot::ItemKind => {
				let kind: Cow<'_, str> = serde_json::from_slice(&self.kept)?;
				self.item.reasoning = kind == "reasoning";
			},
			_ => {},
		}
		Ok(())
	}
}

const fn malformed(what: &'static str) -> DecodeError {
	DecodeError::Malformed(what)
}

#[cfg(test)]
#[path = "terminal_scan_tests.rs"]
mod tests;
