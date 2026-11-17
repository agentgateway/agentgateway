//! Protocol-neutral reply events and observers for usage, telemetry, and replay state.
//!
//! Events borrow decoded payloads. Consumers copy values they retain beyond the
//! decoder callback. Provider-reported errors use [`Event::Error`]; malformed or
//! unsupported input returns [`DecodeError`].

use std::borrow::Cow;
use std::str::Utf8Error;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::ir::events;
use crate::json::{decode_string, is_strict};

/// Parts are numbered in the order they start, across the whole reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartId(pub u32);

/// A JSON string exactly as the provider wrote it, escapes included, so it can
/// be forwarded without being decoded and escaped again. It always decodes.
#[derive(Debug, Clone, Copy)]
pub struct JsonStr<'a>(pub(crate) &'a RawValue);

impl<'a> JsonStr<'a> {
	#[must_use]
	pub fn new(raw: &'a RawValue) -> Option<Self> {
		Self::read(raw, "string").ok()
	}

	/// `what` names the value in the error.
	pub fn read(raw: &'a RawValue, what: &'static str) -> Result<Self, DecodeError> {
		let text = raw.get();
		if !text.starts_with('"') {
			return Err(DecodeError::WrongType(what));
		}
		// `RawValue` has checked the rest of the string, so only an escape can fail.
		if text.contains("\\u") {
			decode_text(text)?;
		}
		Ok(Self(raw))
	}

	#[must_use]
	pub fn raw(&self) -> &'a str {
		self.0.get()
	}

	pub(crate) const fn raw_value(&self) -> &'a RawValue {
		self.0
	}

	/// Returns decoded text, borrowing the input when it contains no escapes.
	#[must_use]
	pub fn decode(&self) -> Cow<'a, str> {
		decode_string(self.0.get())
	}

	/// `decode().len()`, counted without building the text.
	#[must_use]
	pub fn decoded_len(&self) -> usize {
		decoded_len(self.0.get())
	}
}

/// The decoded length of a checked JSON string, counted without building the text.
pub(crate) fn decoded_len(raw: &str) -> usize {
	let inside = &raw[1..raw.len() - 1];
	let mut len = inside.len();
	let mut rest = inside;
	while let Some(at) = rest.find('\\') {
		let escape = &rest[at..];
		let (written, decoded) = match escape.as_bytes()[1] {
			b'u' => {
				let unit = u32::from_str_radix(&escape[2..6], 16)
					.unwrap_or_else(|_| unreachable!("a checked string has four hex digits"));
				match unit {
					// A surrogate pair, `\uXXXX\uXXXX`, is one four-byte character.
					0xD800..=0xDBFF => (12, 4),
					..0x80 => (6, 1),
					0x80..0x800 => (6, 2),
					_ => (6, 3),
				}
			},
			_ => (2, 1),
		};
		len -= written - decoded;
		rest = &escape[written..];
	}
	len
}

fn decode_text(raw: &str) -> Result<Cow<'_, str>, serde_json::Error> {
	#[derive(Deserialize)]
	struct Text<'a>(#[serde(borrow)] Cow<'a, str>);
	let Text(text) = serde_json::from_str(raw)?;
	Ok(text)
}

/// A tool call's arguments as one JSON string, escapes included.
#[derive(Debug, Clone, Copy)]
pub enum Arguments<'a> {
	Complete(CompleteArguments<'a>),
	/// The provider stopped inside the call, so they may not be JSON.
	Partial(JsonStr<'a>),
}

/// Strict JSON, or empty for no arguments. A caller makes one only through
/// `Arguments::complete`.
#[derive(Debug, Clone, Copy)]
pub struct CompleteArguments<'a>(pub(crate) JsonStr<'a>);

impl<'a> Arguments<'a> {
	#[must_use]
	pub fn complete(value: JsonStr<'a>) -> Option<Self> {
		let text = value.decode();
		(text.is_empty() || is_strict(&text)).then_some(Self::Complete(CompleteArguments(value)))
	}

	#[must_use]
	pub fn raw(&self) -> &'a str {
		match self {
			Self::Complete(CompleteArguments(value)) | Self::Partial(value) => value.raw(),
		}
	}

	pub(crate) const fn raw_value(&self) -> &'a RawValue {
		match self {
			Self::Complete(CompleteArguments(value)) | Self::Partial(value) => value.raw_value(),
		}
	}

	#[must_use]
	pub const fn is_complete(&self) -> bool {
		matches!(self, Self::Complete(_))
	}
}

/// The kind and identity of a reply part when it starts.
#[derive(Debug, Clone, Copy)]
pub enum PartKind<'a> {
	Text,
	Refusal,
	Reasoning,
	ToolCall { call_id: &'a str, name: &'a str },
}

/// The complete value attached to the end of a reply part.
#[derive(Debug, Clone, Copy)]
pub enum Final<'a> {
	/// Text is exactly its deltas, so its final value has none: a consumer
	/// that needs the whole text keeps the deltas itself. A reply with no deltas,
	/// such as a buffered one, sends its text as one.
	Text,
	Refusal(JsonStr<'a>),
	ToolCall(Arguments<'a>),
	Reasoning {
		/// `None` when the provider hides it.
		text: Option<JsonStr<'a>>,
		/// Opaque replay state. Empty `text` doesn't mean there is none.
		encrypted: Option<JsonStr<'a>>,
		/// The provider's whole item, byte for byte.
		item: &'a RawValue,
	},
}

/// A part's text gathered from its deltas, written as one JSON string.
pub(crate) struct OpenText {
	raw: String,
}

impl Default for OpenText {
	fn default() -> Self {
		Self {
			raw: String::from('"'),
		}
	}
}

impl OpenText {
	pub(crate) fn push(&mut self, text: &str) {
		let encoded = serde_json::to_string(text)
			.unwrap_or_else(|error| unreachable!("a text delta serializes: {error}"));
		self.raw.push_str(&encoded[1..encoded.len() - 1]);
	}

	/// Keeps the provider's escapes. Each checked string is whole, so they join
	/// into one valid string.
	pub(crate) fn push_json(&mut self, text: JsonStr<'_>) {
		let raw = text.raw();
		self.raw.push_str(&raw[1..raw.len() - 1]);
	}

	pub(crate) const fn is_empty(&self) -> bool {
		self.raw.len() == 1
	}

	/// The length of the finished JSON string.
	pub(crate) const fn len(&self) -> usize {
		self.raw.len() + 1
	}

	pub(crate) fn finish(mut self) -> String {
		self.raw.push('"');
		self.raw
	}
}

/// Tokens as the provider reported them. `None` means unknown, never zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
	pub input_tokens: Option<u64>,
	pub output_tokens: Option<u64>,
	pub total_tokens: Option<u64>,
	pub cached_tokens: Option<u64>,
	pub cache_write_tokens: Option<u64>,
	/// The part of `cache_write_tokens` kept for 12 hours, which costs more.
	pub cache_write_12h_tokens: Option<u64>,
	pub reasoning_tokens: Option<u64>,
}

/// Why a reply stopped early, in words every writer understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
	MaxTokens,
	ContentFilter,
	ContextWindow,
	/// Nothing a client can act on.
	Other,
}

/// Whether the provider completed the reply or stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome<'a> {
	Completed,
	Incomplete {
		reason: StopReason,
		/// The provider's own word, for logs.
		raw: Option<&'a str>,
	},
}

/// What went wrong, in words every writer understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
	ContextTooLong,
	InvalidRequest,
	RequestTooLarge,
	Authentication,
	Permission,
	NotFound,
	Billing,
	RateLimit,
	Overloaded,
	Unknown,
}

/// An error that ends a reply, classified for client-protocol encoding: the provider's,
/// or the caller's when it stops the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderError<'a> {
	pub kind: ErrorKind,
	/// The provider's raw values, for logs. Clients never see them.
	pub code: Option<&'a str>,
	pub message: Option<&'a str>,
	/// What the client is told in place of its protocol's fixed text for `kind`. Only the
	/// caller sets it, for a reply it stopped; a provider's words never reach a client.
	pub client_message: Option<&'a str>,
}

/// One protocol-neutral event decoded from a provider reply.
#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
	ResponseStart {
		id: &'a str,
		model: &'a str,
	},
	PartStart {
		part: PartId,
		kind: PartKind<'a>,
	},
	/// More of a part: text, reasoning text or tool arguments.
	Delta {
		part: PartId,
		text: JsonStr<'a>,
	},
	PartEnd {
		part: PartId,
		value: Final<'a>,
	},
	/// A source for a text part. It can arrive at any point in the part.
	Citation {
		part: PartId,
		url: &'a str,
		title: Option<&'a str>,
	},
	/// A whole assistant message, byte for byte, for fields such as `phase`
	/// that have no event of their own.
	MessageEnd {
		item: &'a RawValue,
	},
	/// The provider's raw processing class. Comes before `Usage`.
	ServiceTier(&'a str),
	/// The stop sequence that ended the reply, when the provider names it. Comes
	/// before `Finish`, which is still `Completed`.
	StopSequence(&'a str),
	Usage(Usage),
	Finish(Outcome<'a>),
	Error(ProviderError<'a>),
}

pub const MAX_PARTS: usize = 4096;

/// A malformed or unsupported provider reply.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
	#[error("invalid JSON from provider: {0}")]
	InvalidJson(#[from] serde_json::Error),
	#[error("provider data is not UTF-8")]
	NotUtf8(#[from] Utf8Error),
	/// JSON the piecewise scanner refused, which has no serde error to keep.
	#[error("malformed JSON from provider: {0}")]
	Malformed(&'static str),
	#[error("unknown provider event {0}")]
	UnknownEvent(String),
	#[error("out of order: {0}")]
	OutOfOrder(&'static str),
	#[error("wrong type: {0}")]
	WrongType(&'static str),
	#[error("unsupported output item {0}")]
	UnsupportedItem(String),
	#[error("final {0} differs from what was streamed")]
	FinalDiffers(&'static str),
	#[error("{0} arrived only at the end")]
	LateOutput(&'static str),
	#[error("final tool arguments are not valid JSON")]
	InvalidArguments,
	#[error("more than {MAX_PARTS} parts")]
	TooManyParts,
	#[error("stream ended with no terminal event")]
	NoTerminalEvent,
	#[error("non-terminal event over the {limit} byte limit")]
	EventTooLarge { limit: usize },
}

#[derive(Default)]
pub(crate) struct PartIds {
	next: u32,
}

impl PartIds {
	pub(crate) const fn allocate(&mut self) -> Result<PartId, DecodeError> {
		if self.next as usize >= MAX_PARTS {
			return Err(DecodeError::TooManyParts);
		}
		self.next += 1;
		Ok(PartId(self.next - 1))
	}
}

/// Keeps the largest reported value for each token count; missing values stay unknown.
/// Repeated usage events do not add tokens. Use a separate instance per provider reply.
#[derive(Default)]
pub struct UsageFold {
	total: Usage,
}

impl UsageFold {
	pub fn event(&mut self, event: &Event<'_>) {
		if let Event::Usage(usage) = event {
			self.merge(*usage);
		}
	}

	pub fn merge(&mut self, usage: Usage) {
		let total = &mut self.total;
		for (kept, seen) in [
			(&mut total.input_tokens, usage.input_tokens),
			(&mut total.output_tokens, usage.output_tokens),
			(&mut total.total_tokens, usage.total_tokens),
			(&mut total.cached_tokens, usage.cached_tokens),
			(&mut total.cache_write_tokens, usage.cache_write_tokens),
			(
				&mut total.cache_write_12h_tokens,
				usage.cache_write_12h_tokens,
			),
			(&mut total.reasoning_tokens, usage.reasoning_tokens),
		] {
			*kept = (*kept).max(seen);
		}
	}

	#[must_use]
	pub const fn total(&self) -> Usage {
		self.total
	}
}

/// Provider reply facts collected for logs and metrics.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Telemetry {
	pub response_id: Option<String>,
	pub model: Option<String>,
	pub service_tier: Option<String>,
	pub parts: u32,
	pub tool_calls: u32,
	pub deltas: u64,
	/// The provider's raw finish reason, or `completed`.
	pub finish: Option<String>,
	pub error_code: Option<String>,
	/// Never sent to a client.
	pub error_message: Option<String>,
	pub failed: bool,
}

impl Telemetry {
	pub fn event(&mut self, event: &Event<'_>) {
		match event {
			Event::ResponseStart { id, model } => {
				self.response_id = Some((*id).to_string());
				self.model = Some((*model).to_string());
			},
			Event::PartStart { kind, .. } => {
				self.parts += 1;
				if matches!(kind, events::PartKind::ToolCall { .. }) {
					self.tool_calls += 1;
				}
			},
			Event::Delta { .. } => self.deltas += 1,
			Event::ServiceTier(tier) => self.service_tier = Some((*tier).to_string()),
			Event::StopSequence(_) => {},
			Event::Finish(Outcome::Completed) => self.finish = Some("completed".to_string()),
			Event::Finish(Outcome::Incomplete { raw, .. }) => {
				self.finish = Some(raw.unwrap_or("incomplete").to_string());
			},
			Event::Error(error) => {
				self.failed = true;
				self.error_code = error.code.map(str::to_string);
				self.error_message = error.message.map(str::to_string);
			},
			_ => {},
		}
	}
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
