//! A stream's `delta` and a reply's `message` have the same shape, so both take
//! the same path here. Reasoning keeps the field names the engine used, so a
//! later turn can send it back as it came.
//!
//! A stream ends with `[DONE]`, and usage may come after `finish_reason`, so the
//! ending waits for it. A stream without both is a failure.

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::ir::events::{
	Arguments, DecodeError, ErrorKind, Event, Final, JsonStr, OpenText, Outcome, PartId, PartIds,
	PartKind, ProviderError, StopReason, Usage,
};
use crate::json::{string_value, text_of};
use crate::protocols::openai_error_field;

const DONE: &str = "[DONE]";
/// How engines word a prompt that's too long, when they give no code for it.
const CONTEXT_WORDINGS: [&str; 9] = [
	"prompt is too long",
	"input is too long for requested model",
	"exceeds the context window",
	"exceed context limit",
	"maximum context length is",
	"model's maximum context limit",
	"is longer than the model's context length",
	"input tokens exceed the configured limit",
	"exceeds the available context size",
];

pub struct CompletionsDecoder {
	requested_model: String,
	/// The id of a reply that names none.
	fallback_reply_id: String,
	id: String,
	started: bool,
	finished: bool,
	parts: PartIds,
	open: Option<Open>,
	calls: Vec<Call>,
	finish_reason: Option<String>,
	stop_sequence: Option<String>,
	draining: bool,
}

struct Open {
	part: PartId,
	content: OpenContent,
	decoded_bytes: usize,
}

enum OpenContent {
	Text,
	Refusal(OpenText),
	Reasoning {
		field: &'static str,
		text: OpenText,
		signature: Option<Box<RawValue>>,
	},
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpenKind {
	Text,
	Refusal,
	Reasoning { field: &'static str },
}

struct Call {
	index: u32,
	/// `None` until its name has arrived.
	part: Option<PartId>,
	id: Option<String>,
	name: Option<String>,
	arguments: String,
}

impl CompletionsDecoder {
	/// Replies don't always name their model or themselves, so these fill in for them.
	/// `fallback_reply_id` must be unique per reply: call ids are made up from it.
	#[must_use]
	pub fn new(requested_model: &str, fallback_reply_id: &str) -> Self {
		Self {
			requested_model: requested_model.to_string(),
			fallback_reply_id: fallback_reply_id.to_string(),
			id: String::new(),
			started: false,
			finished: false,
			parts: PartIds::default(),
			open: None,
			calls: Vec::new(),
			finish_reason: None,
			stop_sequence: None,
			draining: false,
		}
	}

	#[must_use]
	pub const fn is_finished(&self) -> bool {
		self.finished
	}

	/// Takes one payload after the caller removes SSE framing.
	/// `max_block_bytes` bounds one chunk, and each part built from chunks.
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
		if data.trim_ascii() == DONE.as_bytes() {
			self.finished = true;
			return;
		}
		let Ok(reply) = serde_json::from_slice::<WireReply<'_>>(data) else {
			return;
		};
		send_usage(&reply, sink);
		self.finished = reply.error.is_some();
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
		let data = data.trim();
		if data.is_empty() {
			return Ok(());
		}
		if self.finished {
			return Err(DecodeError::OutOfOrder("chunk after the end"));
		}
		if data == DONE {
			return self.done(sink);
		}
		let reply: WireReply<'_> = serde_json::from_str(data)?;
		self.reply(&reply, max_block_bytes, sink)
	}

	fn reply(
		&mut self,
		reply: &WireReply<'_>,
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if let Some(error) = &reply.error {
			self.start(reply, sink);
			self.finished = true;
			sink(Event::Error(provider_error(error)));
			return Ok(());
		}
		// Azure's first chunk reports filters, with no id, model or choices.
		if reply.choices.is_empty() && reply.usage.is_none() {
			return Ok(());
		}
		self.start(reply, sink);
		send_usage(reply, sink);
		let choice = match reply.choices.as_slice() {
			[] => return Ok(()),
			[choice] if choice.index.unwrap_or(0) == 0 => choice,
			_ => return Err(DecodeError::UnsupportedItem("choices".to_string())),
		};
		if self.finish_reason.is_some() {
			return Err(DecodeError::OutOfOrder("choice after finish_reason"));
		}
		if let Some(message) = choice.delta.as_ref().or(choice.message.as_ref()) {
			self.message(message, max_block_bytes, sink)?;
		}
		if let Some(reason) = choice.finish_reason.as_deref() {
			self.close(sink)?;
			self.close_calls(sink)?;
			self.finish_reason = Some(reason.to_string());
			self.stop_sequence = choice.stop_sequence().filter(|_| reason == "stop");
		}
		Ok(())
	}

	fn start(&mut self, reply: &WireReply<'_>, sink: &mut dyn FnMut(Event<'_>)) {
		if std::mem::replace(&mut self.started, true) {
			return;
		}
		let id = reply.id.as_deref().filter(|id| !id.is_empty());
		self.id = id.unwrap_or(&self.fallback_reply_id).to_string();
		let model = reply.model.as_deref().filter(|model| !model.is_empty());
		sink(Event::ResponseStart {
			id: &self.id,
			model: model.unwrap_or(&self.requested_model),
		});
	}

	fn message(
		&mut self,
		message: &WireMessage<'_>,
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let reasoning = [
			("reasoning_content", message.reasoning_content),
			("reasoning", message.reasoning),
		];
		for (field, raw) in reasoning {
			if let Some(raw) = raw {
				let text = JsonStr::read(raw, "reasoning")?;
				self.delta(OpenKind::Reasoning { field }, text, max_block_bytes, sink)?;
			}
		}
		if let Some(raw) = message.reasoning_signature {
			self.signature(raw, sink)?;
		}
		if let Some(raw) = message.refusal {
			let text = JsonStr::read(raw, "refusal")?;
			self.delta(OpenKind::Refusal, text, max_block_bytes, sink)?;
		}
		if let Some(raw) = message.content {
			let text = JsonStr::read(raw, "content")?;
			self.delta(OpenKind::Text, text, max_block_bytes, sink)?;
		}
		for call in message.tool_calls.iter().flatten() {
			self.close(sink)?;
			self.call(call, max_block_bytes, sink)?;
		}
		Ok(())
	}

	/// Withheld reasoning is a signature with no text, so this can open a part.
	fn signature(
		&mut self,
		raw: &RawValue,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		JsonStr::read(raw, "reasoning_signature")?;
		if raw.get() == r#""""# {
			return Ok(());
		}
		let reasoning = |open: &Open| matches!(open.content, OpenContent::Reasoning { .. });
		if !self.open.as_ref().is_some_and(reasoning) {
			self.close(sink)?;
			self.open_part(
				OpenKind::Reasoning {
					field: "reasoning_content",
				},
				sink,
			)?;
		}
		let Some(Open {
			content: OpenContent::Reasoning { signature, .. },
			..
		}) = &mut self.open
		else {
			unreachable!("a reasoning part was opened for the signature");
		};
		*signature = Some(raw.to_owned());
		Ok(())
	}

	fn delta(
		&mut self,
		kind: OpenKind,
		text: JsonStr<'_>,
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if text.raw() == r#""""# {
			return Ok(());
		}
		if self
			.open
			.as_ref()
			.is_some_and(|open| open.content.kind() != kind)
		{
			self.close(sink)?;
		}
		if self.open.is_none() {
			self.open_part(kind, sink)?;
		}
		let Some(open) = &mut self.open else {
			return Err(DecodeError::OutOfOrder("delta outside a part"));
		};
		open.decoded_bytes += text.decoded_len();
		match &mut open.content {
			OpenContent::Text => {},
			OpenContent::Refusal(kept) | OpenContent::Reasoning { text: kept, .. } => {
				kept.push(&text.decode());
			},
		}
		if open.decoded_bytes > max_block_bytes {
			return Err(DecodeError::EventTooLarge {
				limit: max_block_bytes,
			});
		}
		sink(Event::Delta {
			part: open.part,
			text,
		});
		Ok(())
	}

	fn open_part(
		&mut self,
		kind: OpenKind,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let part = self.parts.allocate()?;
		sink(Event::PartStart {
			part,
			kind: match kind {
				OpenKind::Text => PartKind::Text,
				OpenKind::Refusal => PartKind::Refusal,
				OpenKind::Reasoning { .. } => PartKind::Reasoning,
			},
		});
		self.open = Some(Open {
			part,
			content: match kind {
				OpenKind::Text => OpenContent::Text,
				OpenKind::Refusal => OpenContent::Refusal(OpenText::default()),
				OpenKind::Reasoning { field } => OpenContent::Reasoning {
					field,
					text: OpenText::default(),
					signature: None,
				},
			},
			decoded_bytes: 0,
		});
		Ok(())
	}

	fn close(&mut self, sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
		let Some(open) = self.open.take() else {
			return Ok(());
		};
		match open.content {
			OpenContent::Text => sink(Event::PartEnd {
				part: open.part,
				value: Final::Text,
			}),
			OpenContent::Refusal(kept) => {
				let raw = kept.finish();
				let text = JsonStr::read(serde_json::from_str(&raw)?, "text")?;
				sink(Event::PartEnd {
					part: open.part,
					value: Final::Refusal(text),
				});
			},
			OpenContent::Reasoning {
				field,
				text: kept,
				signature,
			} => {
				let empty = kept.is_empty();
				let raw = kept.finish();
				let text = JsonStr::read(serde_json::from_str(&raw)?, "text")?;
				let signature = signature.as_deref();
				let item = reasoning_item(field, text, signature)?;
				sink(Event::PartEnd {
					part: open.part,
					value: Final::Reasoning {
						text: (!empty).then_some(text),
						encrypted: signature
							.map(|raw| JsonStr::read(raw, "signature"))
							.transpose()?,
						item: &item,
					},
				});
			},
		}
		Ok(())
	}

	fn call(
		&mut self,
		wire: &WireCall<'_>,
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if wire.custom.is_some() {
			return Err(DecodeError::UnsupportedItem("custom tool call".to_string()));
		}
		let index = match wire.index {
			Some(index) => index,
			None => u32::try_from(self.calls.len()).map_err(|_| DecodeError::TooManyParts)?,
		};
		let at = match self.calls.iter().position(|call| call.index == index) {
			Some(at) => at,
			None => {
				self.calls.push(Call {
					index,
					part: None,
					id: None,
					name: None,
					arguments: String::new(),
				});
				self.calls.len() - 1
			},
		};
		let function = wire.function.as_ref();
		let arguments = function.and_then(|function| function.arguments);
		let arguments = arguments
			.map(|raw| JsonStr::read(raw, "arguments"))
			.transpose()?;
		let call = &mut self.calls[at];
		if call.id.is_none() {
			call.id = wire.id.as_deref().map(str::to_string);
		}
		if call.name.is_none() {
			call.name = function
				.and_then(|function| function.name.as_deref())
				.map(str::to_string);
		}
		let waiting = call.part.is_none();
		if let Some(arguments) = arguments {
			call.arguments.push_str(&arguments.decode());
			if call.arguments.len() > max_block_bytes {
				return Err(DecodeError::EventTooLarge {
					limit: max_block_bytes,
				});
			}
		}
		if waiting {
			self.open_call(at, sink)
		} else {
			if let (Some(part), Some(text)) = (self.calls[at].part, arguments) {
				sink(Event::Delta { part, text });
			}
			Ok(())
		}
	}

	/// Arguments can come before a name, so they go out once the call can start.
	fn open_call(&mut self, at: usize, sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
		let call = &self.calls[at];
		let Some(name) = &call.name else {
			return Ok(());
		};
		let call_id = match &call.id {
			Some(id) => Cow::Borrowed(id.as_str()),
			None => Cow::Owned(format!("call_{}_{}", self.id, call.index)),
		};
		let part = self.parts.allocate()?;
		sink(Event::PartStart {
			part,
			kind: PartKind::ToolCall {
				call_id: &call_id,
				name,
			},
		});
		if !call.arguments.is_empty() {
			let raw = string_value(&call.arguments)?;
			sink(Event::Delta {
				part,
				text: JsonStr::read(&raw, "arguments")?,
			});
		}
		self.calls[at].part = Some(part);
		Ok(())
	}

	fn close_calls(&mut self, sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
		for call in std::mem::take(&mut self.calls) {
			let Some(part) = call.part else {
				return Err(DecodeError::WrongType("tool call without a name"));
			};
			let raw = string_value(&call.arguments)?;
			let value = JsonStr::read(&raw, "arguments")?;
			sink(Event::PartEnd {
				part,
				value: Final::ToolCall(Arguments::complete(value).unwrap_or(Arguments::Partial(value))),
			});
		}
		Ok(())
	}

	fn done(&mut self, sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
		let Some(reason) = self.finish_reason.take() else {
			return Err(DecodeError::NoTerminalEvent);
		};
		self.finished = true;
		if let Some(sequence) = &self.stop_sequence {
			sink(Event::StopSequence(sequence));
		}
		sink(ending(&reason));
		Ok(())
	}
}

impl OpenContent {
	const fn kind(&self) -> OpenKind {
		match self {
			Self::Text => OpenKind::Text,
			Self::Refusal(_) => OpenKind::Refusal,
			Self::Reasoning { field, .. } => OpenKind::Reasoning { field },
		}
	}
}

/// Usage is sent as it comes, because a stream can stop at any point and tokens are
/// spent by then.
fn send_usage(reply: &WireReply<'_>, sink: &mut dyn FnMut(Event<'_>)) {
	if let Some(tier) = reply.service_tier.as_deref() {
		sink(Event::ServiceTier(tier));
	}
	if let Some(usage) = &reply.usage {
		sink(Event::Usage(usage.usage()));
	}
}

fn reasoning_item(
	field: &str,
	text: JsonStr<'_>,
	signature: Option<&RawValue>,
) -> Result<Box<RawValue>, DecodeError> {
	let mut item = String::from("{\"");
	item.push_str(field);
	item.push_str("\":");
	item.push_str(text.raw());
	if let Some(signature) = signature {
		item.push_str(",\"reasoning_signature\":");
		item.push_str(signature.get());
	}
	item.push('}');
	Ok(RawValue::from_string(item)?)
}

/// An unknown reason fails, because it may mean the reply can't be trusted.
fn ending(reason: &str) -> Event<'_> {
	let incomplete = |kind| {
		Event::Finish(Outcome::Incomplete {
			reason: kind,
			raw: Some(reason),
		})
	};
	match reason {
		"stop" | "tool_calls" | "function_call" => Event::Finish(Outcome::Completed),
		"length" => incomplete(StopReason::MaxTokens),
		"content_filter" => incomplete(StopReason::ContentFilter),
		_ => Event::Error(ProviderError {
			kind: ErrorKind::Unknown,
			code: Some(reason),
			message: None,
			client_message: None,
		}),
	}
}

/// Any body is accepted, JSON or not.
#[must_use]
pub fn http_error_kind(body: &[u8]) -> ErrorKind {
	#[derive(Deserialize)]
	struct Wire<'a> {
		#[serde(borrow)]
		error: WireError<'a>,
	}
	serde_json::from_slice::<Wire<'_>>(body)
		.map_or(ErrorKind::Unknown, |wire| error_kind(&wire.error))
}

#[must_use]
pub fn http_error_field(body: &[u8]) -> Option<String> {
	openai_error_field(body)
}

/// Gives the same events as a stream.
pub fn decode_buffered(
	body: &[u8],
	requested_model: &str,
	fallback_reply_id: &str,
	sink: &mut dyn FnMut(Event<'_>),
) -> Result<(), DecodeError> {
	let reply: WireReply<'_> = serde_json::from_slice(body)?;
	let mut decoder = CompletionsDecoder::new(requested_model, fallback_reply_id);
	decoder.reply(&reply, usize::MAX, sink)?;
	if !decoder.finished {
		decoder.done(sink)?;
	}
	Ok(())
}

fn provider_error<'a>(error: &'a WireError<'a>) -> ProviderError<'a> {
	ProviderError {
		kind: error_kind(error),
		code: error.code.map(|code| code.get().trim_matches('"')),
		message: error.message.as_deref(),
		client_message: None,
	}
}

fn error_kind(error: &WireError<'_>) -> ErrorKind {
	let code = error.code.and_then(text_of);
	let kind = error.kind.as_deref();
	match code.as_deref().or(kind) {
		Some("context_length_exceeded") => ErrorKind::ContextTooLong,
		// Many engines have no code for a prompt that's too long, so only wording shows it.
		_ if too_long(error.message.as_deref().unwrap_or_default()) => ErrorKind::ContextTooLong,
		Some("insufficient_quota" | "billing_not_active") => ErrorKind::Billing,
		Some("rate_limit_exceeded" | "rate_limit_error") => ErrorKind::RateLimit,
		Some("model_not_found") => ErrorKind::NotFound,
		Some("invalid_api_key" | "authentication_error") => ErrorKind::Authentication,
		Some("permission_error") => ErrorKind::Permission,
		Some("server_is_overloaded" | "overloaded_error") => ErrorKind::Overloaded,
		Some("invalid_request_error") => ErrorKind::InvalidRequest,
		_ => match kind {
			Some("invalid_request_error") => ErrorKind::InvalidRequest,
			_ => ErrorKind::Unknown,
		},
	}
}

fn too_long(message: &str) -> bool {
	let message = message.to_ascii_lowercase();
	CONTEXT_WORDINGS.iter().any(|words| message.contains(words))
}

#[derive(Deserialize)]
struct WireReply<'a> {
	#[serde(borrow)]
	id: Option<Cow<'a, str>>,
	#[serde(borrow)]
	model: Option<Cow<'a, str>>,
	#[serde(default, borrow)]
	choices: Vec<WireChoice<'a>>,
	usage: Option<WireUsage>,
	#[serde(borrow)]
	service_tier: Option<Cow<'a, str>>,
	#[serde(borrow)]
	error: Option<WireError<'a>>,
}

#[derive(Deserialize)]
struct WireChoice<'a> {
	index: Option<u32>,
	#[serde(borrow)]
	delta: Option<WireMessage<'a>>,
	#[serde(borrow)]
	message: Option<WireMessage<'a>>,
	#[serde(borrow)]
	finish_reason: Option<Cow<'a, str>>,
	/// vLLM's name for the stop sequence that matched.
	#[serde(borrow)]
	stop_reason: Option<&'a RawValue>,
	/// SGLang's name for it.
	#[serde(borrow)]
	matched_stop: Option<&'a RawValue>,
}

impl WireChoice<'_> {
	/// Engines can give a number here, a token that isn't a sequence the client asked for.
	fn stop_sequence(&self) -> Option<String> {
		[self.stop_reason, self.matched_stop]
			.into_iter()
			.flatten()
			.filter_map(text_of)
			.find(|text| !text.is_empty())
			.map(Cow::into_owned)
	}
}

#[derive(Deserialize)]
struct WireMessage<'a> {
	#[serde(borrow)]
	content: Option<&'a RawValue>,
	#[serde(borrow)]
	refusal: Option<&'a RawValue>,
	#[serde(borrow)]
	reasoning_content: Option<&'a RawValue>,
	#[serde(borrow)]
	reasoning: Option<&'a RawValue>,
	#[serde(borrow)]
	reasoning_signature: Option<&'a RawValue>,
	#[serde(borrow)]
	tool_calls: Option<Vec<WireCall<'a>>>,
}

#[derive(Deserialize)]
struct WireCall<'a> {
	index: Option<u32>,
	#[serde(borrow)]
	id: Option<Cow<'a, str>>,
	#[serde(borrow)]
	function: Option<WireFunction<'a>>,
	#[serde(borrow)]
	custom: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct WireFunction<'a> {
	#[serde(borrow)]
	name: Option<Cow<'a, str>>,
	#[serde(borrow)]
	arguments: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct WireError<'a> {
	#[serde(borrow)]
	message: Option<Cow<'a, str>>,
	#[serde(rename = "type", borrow)]
	kind: Option<Cow<'a, str>>,
	/// A string on OpenAI, and a number on some engines.
	#[serde(borrow)]
	code: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct WireUsage {
	prompt_tokens: Option<u64>,
	completion_tokens: Option<u64>,
	total_tokens: Option<u64>,
	cache_read_input_tokens: Option<u64>,
	cache_creation_input_tokens: Option<u64>,
	prompt_tokens_details: Option<WirePromptDetails>,
	completion_tokens_details: Option<WireCompletionDetails>,
}

#[derive(Deserialize)]
struct WirePromptDetails {
	cached_tokens: Option<u64>,
	cache_write_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct WireCompletionDetails {
	reasoning_tokens: Option<u64>,
}

impl WireUsage {
	fn usage(&self) -> Usage {
		let prompt = self.prompt_tokens_details.as_ref();
		let completion = self.completion_tokens_details.as_ref();
		// Gemini's OpenAI-compatible endpoint leaves reasoning out of `completion_tokens` but
		// counts it in `total_tokens`, so the difference is output too.
		let beyond_prompt = self
			.total_tokens
			.zip(self.prompt_tokens)
			.map(|(total, prompt)| total.saturating_sub(prompt));
		Usage {
			input_tokens: self.prompt_tokens,
			output_tokens: self.completion_tokens.max(beyond_prompt),
			total_tokens: self.total_tokens,
			cached_tokens: prompt
				.and_then(|details| details.cached_tokens)
				.or(self.cache_read_input_tokens),
			cache_write_tokens: prompt
				.and_then(|details| details.cache_write_tokens)
				.or(self.cache_creation_input_tokens),
			cache_write_12h_tokens: None,
			reasoning_tokens: completion.and_then(|details| details.reasoning_tokens),
		}
	}
}

#[cfg(test)]
#[path = "decoder_tests.rs"]
mod tests;
