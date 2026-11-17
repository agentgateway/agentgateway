//! Messages sends no final values, so this decoder builds them from what it
//! streamed. Reasoning state is the provider's thinking block, kept whole so a
//! later turn can send it back. Blocks can't overlap. A token limit can end an
//! open tool block; its arguments remain incomplete even if they are valid JSON.

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::ir::events::{
	Arguments, DecodeError, ErrorKind, Event, Final, JsonStr, MAX_PARTS, Outcome, PartId, PartIds,
	PartKind, ProviderError, StopReason, Usage,
};

#[derive(Default)]
pub struct MessagesDecoder {
	started: bool,
	finished: bool,
	next_index: u32,
	parts: PartIds,
	open: Option<(PartId, Open)>,
	/// A stopped call waits for the next block or the message's stop reason.
	pending_call: Option<(PartId, String)>,
	usage: WireUsage,
	stop_reason: Option<String>,
	draining: bool,
}

impl MessagesDecoder {
	#[must_use]
	pub const fn is_finished(&self) -> bool {
		self.finished
	}

	/// Takes one payload after the caller removes SSE framing.
	/// `max_block_bytes` bounds one event, and each block built from its deltas.
	/// After an error, continue until `is_finished` or end of input to recover usage.
	/// The client reply must remain failed.
	pub fn push_json(
		&mut self,
		data: &[u8],
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if self.draining && !self.finished {
			if data.len() <= max_block_bytes {
				self.drain(data, sink);
			}
			return Ok(());
		}
		let result = self.read(data, max_block_bytes, sink);
		self.draining |= result.is_err();
		result
	}

	pub const fn finish(&self) -> Result<(), DecodeError> {
		if self.finished {
			Ok(())
		} else {
			Err(DecodeError::NoTerminalEvent)
		}
	}

	fn drain(&mut self, data: &[u8], sink: &mut dyn FnMut(Event<'_>)) {
		let Ok(wire) = serde_json::from_slice::<WireEvent<'_>>(data) else {
			return;
		};
		match wire.kind.as_ref() {
			"message_delta" => {
				if let Some(usage) = wire.usage {
					// The stream has already failed, so a bad count has nowhere to go.
					let _ = self.merge_usage(usage, sink);
				}
			},
			"message_stop" | "error" => self.finished = true,
			_ => {},
		}
	}

	fn read(
		&mut self,
		data: &[u8],
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if data.len() > max_block_bytes {
			return Err(DecodeError::EventTooLarge {
				limit: max_block_bytes,
			});
		}
		let data = std::str::from_utf8(data)?;
		if data.trim().is_empty() {
			return Ok(());
		}
		let wire: WireEvent<'_> = serde_json::from_str(data)?;
		if self.finished {
			return Err(DecodeError::OutOfOrder("event after message_stop"));
		}
		match wire.kind.as_ref() {
			"ping" => Ok(()),
			"error" => self.error(wire, sink),
			"message_start" => self.message_start(wire, sink),
			_ if !self.started => Err(DecodeError::OutOfOrder("event before message_start")),
			"content_block_start" => self.block_start(wire, sink),
			"content_block_delta" => self.block_delta(wire, max_block_bytes, sink),
			"content_block_stop" => self.block_stop(&wire, sink),
			"message_delta" => self.message_delta(wire, sink),
			"message_stop" => self.message_stop(sink),
			other => Err(DecodeError::UnknownEvent(other.to_string())),
		}
	}

	fn error(
		&mut self,
		wire: WireEvent<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let error = wire.error.ok_or(DecodeError::WrongType("error"))?;
		self.finished = true;
		sink(Event::Error(ProviderError {
			kind: error_kind(error.kind.as_deref(), error.message.as_deref()),
			code: error.kind.as_deref(),
			message: error.message.as_deref(),
			client_message: None,
		}));
		Ok(())
	}

	fn message_start(
		&mut self,
		wire: WireEvent<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if std::mem::replace(&mut self.started, true) {
			return Err(DecodeError::OutOfOrder("second message_start"));
		}
		let message = wire.message.ok_or(DecodeError::WrongType("message"))?;
		sink(Event::ResponseStart {
			id: &message.id,
			model: &message.model,
		});
		if let Some(usage) = message.usage {
			self.merge_usage(usage, sink)?;
		}
		Ok(())
	}

	fn block_start(
		&mut self,
		wire: WireEvent<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if self.open.is_some() {
			return Err(DecodeError::OutOfOrder("block started inside a block"));
		}
		if self.stop_reason.is_some() {
			return Err(DecodeError::OutOfOrder("block after message_delta"));
		}
		if wire.index != Some(self.next_index) {
			return Err(DecodeError::OutOfOrder("block index"));
		}
		self.next_index += 1;
		let block = wire
			.content_block
			.ok_or(DecodeError::WrongType("content_block"))?;
		if let Some((part, arguments)) = self.pending_call.take() {
			end_call(part, &arguments, false, sink)?;
		}
		let part = self.parts.allocate()?;
		let open = match block.kind.as_ref() {
			"text" => {
				if !is_empty_string(block.text) {
					return Err(DecodeError::WrongType("text at block start"));
				}
				sink(Event::PartStart {
					part,
					kind: PartKind::Text,
				});
				Open::Text { decoded_bytes: 0 }
			},
			"thinking" => {
				sink(Event::PartStart {
					part,
					kind: PartKind::Reasoning,
				});
				Open::Thinking {
					text: String::new(),
					signature: None,
				}
			},
			"redacted_thinking" => {
				sink(Event::PartStart {
					part,
					kind: PartKind::Reasoning,
				});
				Open::Redacted {
					item: redacted_item(&block)?,
				}
			},
			"tool_use" => {
				let (Some(call_id), Some(name)) = (block.id.as_deref(), block.name.as_deref()) else {
					return Err(DecodeError::WrongType("tool_use"));
				};
				if block.input.is_some_and(|input| input.get().trim() != "{}") {
					return Err(DecodeError::WrongType("input at block start"));
				}
				sink(Event::PartStart {
					part,
					kind: PartKind::ToolCall { call_id, name },
				});
				Open::Call {
					arguments: String::new(),
				}
			},
			other => return Err(DecodeError::UnsupportedItem(other.to_string())),
		};
		self.open = Some((part, open));
		Ok(())
	}

	fn block_delta(
		&mut self,
		wire: WireEvent<'_>,
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if wire.index.and_then(|index| index.checked_add(1)) != Some(self.next_index) {
			return Err(DecodeError::OutOfOrder("delta index"));
		}
		let delta = wire.delta.ok_or(DecodeError::WrongType("delta"))?;
		let Some((part, open)) = &mut self.open else {
			return Err(DecodeError::OutOfOrder("delta outside a block"));
		};
		let part = *part;
		let kind = delta.kind.as_deref().unwrap_or_default();
		let (held, raw) = match (kind, open) {
			("text_delta", Open::Text { decoded_bytes }) => (Held::Count(decoded_bytes), delta.text),
			("thinking_delta", Open::Thinking { text, .. }) => (Held::Text(text), delta.thinking),
			("input_json_delta", Open::Call { arguments }) => (Held::Text(arguments), delta.partial_json),
			("signature_delta", Open::Thinking { signature, .. }) => {
				let raw = delta.signature.ok_or(DecodeError::WrongType("signature"))?;
				let value = JsonStr::read(raw, "signature")?;
				*signature = Some(value.decode().into_owned());
				return Ok(());
			},
			("citations_delta", Open::Text { .. }) => {
				let citation = delta.citation.ok_or(DecodeError::WrongType("citation"))?;
				return citation_event(part, &citation, sink);
			},
			(other, _) => return Err(DecodeError::UnsupportedItem(other.to_string())),
		};
		let raw = raw.ok_or(DecodeError::WrongType("delta"))?;
		let value = JsonStr::read(raw, "delta")?;
		let length = match held {
			Held::Count(count) => {
				*count += value.decoded_len();
				*count
			},
			Held::Text(text) => {
				text.push_str(&value.decode());
				text.len()
			},
		};
		if length > max_block_bytes {
			return Err(DecodeError::EventTooLarge {
				limit: max_block_bytes,
			});
		}
		sink(Event::Delta { part, text: value });
		Ok(())
	}

	fn block_stop(
		&mut self,
		wire: &WireEvent<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if wire.index.and_then(|index| index.checked_add(1)) != Some(self.next_index) {
			return Err(DecodeError::OutOfOrder("stop index"));
		}
		let (part, open) = self
			.open
			.take()
			.ok_or(DecodeError::OutOfOrder("stop outside a block"))?;
		match open {
			Open::Call { arguments } => {
				self.pending_call = Some((part, arguments));
				Ok(())
			},
			open => end_block(part, open, sink),
		}
	}

	fn message_delta(
		&mut self,
		wire: WireEvent<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let delta = wire.delta.ok_or(DecodeError::WrongType("delta"))?;
		let cut_off = delta.stop_reason.as_deref() == Some("max_tokens");
		if let Some((part, open)) = self.open.take() {
			match open {
				Open::Call { arguments } if cut_off => end_call(part, &arguments, true, sink)?,
				_ => return Err(DecodeError::OutOfOrder("message_delta inside a block")),
			}
		}
		if let Some((part, arguments)) = self.pending_call.take() {
			end_call(part, &arguments, cut_off, sink)?;
		}
		if let Some(usage) = wire.usage {
			self.merge_usage(usage, sink)?;
		}
		self.stop_reason = delta.stop_reason.map(Cow::into_owned);
		Ok(())
	}

	fn message_stop(&mut self, sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
		if self.open.is_some() {
			return Err(DecodeError::OutOfOrder("message_stop inside a block"));
		}
		let reason = self
			.stop_reason
			.take()
			.ok_or(DecodeError::OutOfOrder("no stop reason"))?;
		self.finished = true;
		sink(Event::Finish(outcome(&reason)));
		Ok(())
	}

	/// Sent as soon as it's known: tokens are spent even if the stream then breaks.
	fn merge_usage(
		&mut self,
		usage: WireUsage,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let kept = &mut self.usage;
		kept.input_tokens = usage.input_tokens.or(kept.input_tokens);
		kept.output_tokens = usage.output_tokens.or(kept.output_tokens);
		kept.cache_read_input_tokens = usage
			.cache_read_input_tokens
			.or(kept.cache_read_input_tokens);
		kept.cache_creation_input_tokens = usage
			.cache_creation_input_tokens
			.or(kept.cache_creation_input_tokens);
		sink(Event::Usage((*kept).try_into()?));
		Ok(())
	}
}

/// Any body is accepted, JSON or not.
#[must_use]
pub fn http_error_kind(body: &[u8]) -> ErrorKind {
	#[derive(Deserialize)]
	struct Wire<'a> {
		#[serde(borrow)]
		error: Inner<'a>,
	}
	#[derive(Deserialize)]
	struct Inner<'a> {
		#[serde(rename = "type", borrow)]
		kind: Option<Cow<'a, str>>,
		#[serde(borrow)]
		message: Option<Cow<'a, str>>,
	}
	let wire = serde_json::from_slice::<Wire<'_>>(body).ok();
	let error = wire.as_ref().map(|wire| &wire.error);
	error_kind(
		error.and_then(|error| error.kind.as_deref()),
		error.and_then(|error| error.message.as_deref()),
	)
}

/// A whole `message` body, as the same events a stream gives.
pub fn decode_buffered(body: &[u8], sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
	let message: WireMessage<'_> = serde_json::from_slice(body)?;
	sink(Event::ResponseStart {
		id: &message.id,
		model: &message.model,
	});
	if let Some(usage) = message.usage {
		sink(Event::Usage(usage.try_into()?));
	}
	if message.content.len() > MAX_PARTS {
		return Err(DecodeError::TooManyParts);
	}
	for (at, block) in message.content.iter().enumerate() {
		let part = PartId(u32::try_from(at).map_err(|_| DecodeError::TooManyParts)?);
		match block.kind.as_ref() {
			"text" => {
				let raw = block.text.ok_or(DecodeError::WrongType("text"))?;
				let text = JsonStr::read(raw, "text")?;
				sink(Event::PartStart {
					part,
					kind: PartKind::Text,
				});
				for citation in block.citations.iter().flatten() {
					citation_event(part, citation, sink)?;
				}
				if text.raw() != EMPTY_TEXT {
					sink(Event::Delta { part, text });
				}
				sink(Event::PartEnd {
					part,
					value: Final::Text,
				});
			},
			"thinking" => {
				let text = string_of(block.thinking)?;
				let signature = string_of(block.signature)?;
				sink(Event::PartStart {
					part,
					kind: PartKind::Reasoning,
				});
				end_block(
					part,
					Open::Thinking {
						text,
						signature: Some(signature),
					},
					sink,
				)?;
			},
			"redacted_thinking" => {
				sink(Event::PartStart {
					part,
					kind: PartKind::Reasoning,
				});
				end_block(
					part,
					Open::Redacted {
						item: redacted_item(block)?,
					},
					sink,
				)?;
			},
			"tool_use" => {
				let (Some(call_id), Some(name), Some(input)) =
					(block.id.as_deref(), block.name.as_deref(), block.input)
				else {
					return Err(DecodeError::WrongType("tool_use"));
				};
				sink(Event::PartStart {
					part,
					kind: PartKind::ToolCall { call_id, name },
				});
				// A reply can't show if its last call was cut off, so it counts as cut off.
				let cut_off =
					message.stop_reason.as_deref() == Some("max_tokens") && at + 1 == message.content.len();
				end_call(part, input.get(), cut_off, sink)?;
			},
			other => return Err(DecodeError::UnsupportedItem(other.to_string())),
		}
	}
	let reason = message
		.stop_reason
		.ok_or(DecodeError::OutOfOrder("no stop reason"))?;
	sink(Event::Finish(outcome(&reason)));
	Ok(())
}

#[derive(Deserialize)]
struct WireEvent<'a> {
	#[serde(rename = "type", borrow)]
	kind: Cow<'a, str>,
	index: Option<u32>,
	#[serde(borrow)]
	message: Option<WireMessage<'a>>,
	#[serde(borrow)]
	content_block: Option<WireBlock<'a>>,
	#[serde(borrow)]
	delta: Option<WireDelta<'a>>,
	usage: Option<WireUsage>,
	#[serde(borrow)]
	error: Option<WireError<'a>>,
}

#[derive(Deserialize)]
struct WireMessage<'a> {
	#[serde(borrow)]
	id: Cow<'a, str>,
	#[serde(borrow)]
	model: Cow<'a, str>,
	#[serde(borrow, default)]
	content: Vec<WireBlock<'a>>,
	#[serde(borrow)]
	stop_reason: Option<Cow<'a, str>>,
	usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireBlock<'a> {
	#[serde(rename = "type", borrow)]
	kind: Cow<'a, str>,
	#[serde(borrow)]
	text: Option<&'a RawValue>,
	#[serde(borrow)]
	thinking: Option<&'a RawValue>,
	#[serde(borrow)]
	signature: Option<&'a RawValue>,
	#[serde(borrow)]
	data: Option<&'a RawValue>,
	#[serde(borrow)]
	id: Option<Cow<'a, str>>,
	#[serde(borrow)]
	name: Option<Cow<'a, str>>,
	#[serde(borrow)]
	input: Option<&'a RawValue>,
	#[serde(borrow, default)]
	citations: Option<Vec<WireCitation<'a>>>,
}

#[derive(Deserialize)]
struct WireDelta<'a> {
	#[serde(rename = "type", borrow)]
	kind: Option<Cow<'a, str>>,
	#[serde(borrow)]
	text: Option<&'a RawValue>,
	#[serde(borrow)]
	thinking: Option<&'a RawValue>,
	#[serde(borrow)]
	signature: Option<&'a RawValue>,
	#[serde(borrow)]
	partial_json: Option<&'a RawValue>,
	#[serde(borrow)]
	citation: Option<WireCitation<'a>>,
	#[serde(borrow)]
	stop_reason: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct WireCitation<'a> {
	#[serde(rename = "type", borrow)]
	kind: Cow<'a, str>,
	#[serde(borrow)]
	url: Option<Cow<'a, str>>,
	#[serde(borrow)]
	title: Option<Cow<'a, str>>,
}

#[derive(Deserialize, Clone, Copy, Default)]
struct WireUsage {
	input_tokens: Option<u64>,
	output_tokens: Option<u64>,
	cache_read_input_tokens: Option<u64>,
	cache_creation_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct WireError<'a> {
	#[serde(rename = "type", borrow)]
	kind: Option<Cow<'a, str>>,
	#[serde(borrow)]
	message: Option<Cow<'a, str>>,
}

enum Open {
	/// Text isn't kept: its deltas are its final value.
	Text {
		decoded_bytes: usize,
	},
	Thinking {
		text: String,
		signature: Option<String>,
	},
	Redacted {
		item: Box<RawValue>,
	},
	Call {
		arguments: String,
	},
}

enum Held<'a> {
	Count(&'a mut usize),
	Text(&'a mut String),
}

const EMPTY_TEXT: &str = r#""""#;

fn end_block(part: PartId, open: Open, sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
	match open {
		Open::Text { .. } => {
			sink(Event::PartEnd {
				part,
				value: Final::Text,
			});
		},
		Open::Thinking { text, signature } => {
			let signature = signature.ok_or(DecodeError::WrongType("no signature"))?;
			let (thinking, signature) = (json_string(&text)?, json_string(&signature)?);
			let item = format!(
				r#"{{"type":"thinking","thinking":{},"signature":{}}}"#,
				thinking.get(),
				signature.get()
			);
			let item = RawValue::from_string(item)?;
			sink(Event::PartEnd {
				part,
				value: Final::Reasoning {
					text: JsonStr::new(&thinking),
					encrypted: JsonStr::new(&signature),
					item: &item,
				},
			});
		},
		Open::Redacted { item } => {
			sink(Event::PartEnd {
				part,
				value: Final::Reasoning {
					text: None,
					encrypted: None,
					item: &item,
				},
			});
		},
		Open::Call { arguments } => {
			end_call(part, &arguments, false, sink)?;
		},
	}
	Ok(())
}

fn end_call(
	part: PartId,
	arguments: &str,
	cut_off: bool,
	sink: &mut dyn FnMut(Event<'_>),
) -> Result<(), DecodeError> {
	let text = if !cut_off && arguments.trim().is_empty() {
		"{}"
	} else {
		arguments
	};
	let raw = json_string(text)?;
	let value = JsonStr::read(&raw, "arguments")?;
	let arguments = if cut_off {
		Arguments::Partial(value)
	} else {
		Arguments::complete(value).unwrap_or(Arguments::Partial(value))
	};
	sink(Event::PartEnd {
		part,
		value: Final::ToolCall(arguments),
	});
	Ok(())
}

fn citation_event(
	part: PartId,
	citation: &WireCitation<'_>,
	sink: &mut dyn FnMut(Event<'_>),
) -> Result<(), DecodeError> {
	match (citation.kind.as_ref(), citation.url.as_deref()) {
		("web_search_result_location", Some(url)) => {
			sink(Event::Citation {
				part,
				url,
				title: citation.title.as_deref(),
			});
			Ok(())
		},
		(other, _) => Err(DecodeError::UnsupportedItem(other.to_string())),
	}
}

fn outcome(reason: &str) -> Outcome<'_> {
	match reason {
		"end_turn" | "tool_use" | "stop_sequence" => Outcome::Completed,
		other => Outcome::Incomplete {
			reason: match other {
				"max_tokens" => StopReason::MaxTokens,
				"refusal" => StopReason::ContentFilter,
				"model_context_window_exceeded" => StopReason::ContextWindow,
				_ => StopReason::Other,
			},
			raw: Some(other),
		},
	}
}

/// Anthropic has no type for a prompt that's too long, so only its wording shows it.
fn error_kind(kind: Option<&str>, message: Option<&str>) -> ErrorKind {
	let too_long = message.is_some_and(|message| message.starts_with("prompt is too long"));
	match kind {
		Some("invalid_request_error") if too_long => ErrorKind::ContextTooLong,
		Some("invalid_request_error") => ErrorKind::InvalidRequest,
		Some("request_too_large") => ErrorKind::RequestTooLarge,
		Some("authentication_error") => ErrorKind::Authentication,
		Some("permission_error") => ErrorKind::Permission,
		Some("not_found_error") => ErrorKind::NotFound,
		Some("billing_error") => ErrorKind::Billing,
		Some("rate_limit_error") => ErrorKind::RateLimit,
		Some("overloaded_error") => ErrorKind::Overloaded,
		_ => ErrorKind::Unknown,
	}
}

fn redacted_item(block: &WireBlock<'_>) -> Result<Box<RawValue>, DecodeError> {
	let data = block.data.ok_or(DecodeError::WrongType("data"))?;
	let item = format!(r#"{{"type":"redacted_thinking","data":{}}}"#, data.get());
	Ok(RawValue::from_string(item)?)
}

fn is_empty_string(raw: Option<&RawValue>) -> bool {
	raw.is_some_and(|raw| raw.get() == "\"\"")
}

fn string_of(raw: Option<&RawValue>) -> Result<String, DecodeError> {
	let raw = raw.ok_or(DecodeError::WrongType("string"))?;
	let value = JsonStr::read(raw, "string")?;
	Ok(value.decode().into_owned())
}

fn json_string(text: &str) -> Result<Box<RawValue>, DecodeError> {
	Ok(RawValue::from_string(serde_json::to_string(text)?)?)
}

/// Responses counts cached tokens inside `input_tokens`; Messages doesn't.
impl TryFrom<WireUsage> for Usage {
	type Error = DecodeError;

	fn try_from(usage: WireUsage) -> Result<Self, DecodeError> {
		let cached = usage.cache_read_input_tokens.unwrap_or(0);
		let written = usage.cache_creation_input_tokens.unwrap_or(0);
		let add = |left: u64, right: u64| {
			left
				.checked_add(right)
				.ok_or(DecodeError::WrongType("usage"))
		};
		let input = usage
			.input_tokens
			.map(|input| add(add(input, cached)?, written))
			.transpose()?;
		let total = input
			.zip(usage.output_tokens)
			.map(|(input, output)| add(input, output))
			.transpose()?;
		Ok(Self {
			input_tokens: input,
			output_tokens: usage.output_tokens,
			total_tokens: total,
			cached_tokens: usage.cache_read_input_tokens,
			cache_write_tokens: usage.cache_creation_input_tokens,
			cache_write_12h_tokens: None,
			reasoning_tokens: None,
		})
	}
}
