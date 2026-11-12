//! `generateContent` replies and `streamGenerateContent` streams are the same
//! JSON, so both take the same path here.
//!
//! Gemini sends whole parts with no boundaries, so a part opens at the first text
//! and closes when the kind changes or a signature arrives. A stream ends at
//! `finishReason`; one without it is a failure.

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::carrier::StateScope;
use crate::ir::events::{
	Arguments, DecodeError, ErrorKind, Event, Final, JsonStr, OpenText, Outcome, PartId, PartIds,
	PartKind, ProviderError, StopReason, Usage,
};
use crate::json::string_value;
use crate::protocols::messages::Failure;
use crate::protocols::messages::writer::{Sequencer, Sink};

pub struct GeminiDecoder {
	requested_model: String,
	/// The id of a reply that names none.
	fallback_reply_id: String,
	id: String,
	started: bool,
	finished: bool,
	parts: PartIds,
	next_call: u32,
	open: Option<Open>,
	tokens: Tokens,
	draining: bool,
}

struct Open {
	part: PartId,
	/// A thought's text, which its final item contains. Answer text isn't kept.
	thought: Option<OpenText>,
	/// Decoded length, which the part limit counts.
	decoded_bytes: usize,
}

impl GeminiDecoder {
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
			next_call: 0,
			open: None,
			tokens: Tokens::default(),
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
		let Ok(reply) = serde_json::from_slice::<WireReply<'_>>(data) else {
			return;
		};
		// The stream has already failed, so a bad count has nowhere to go.
		let _ = self.usage(&reply, sink);
		let feedback = reply.prompt_feedback.as_ref();
		self.finished = reply.error.is_some()
			|| feedback.is_some_and(|feedback| feedback.block_reason.is_some())
			|| reply
				.candidates
				.iter()
				.any(|candidate| candidate.finish_reason.is_some());
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
		let reply: WireReply<'_> = serde_json::from_str(data)?;
		self.reply(&reply, max_block_bytes, sink)
	}

	fn reply(
		&mut self,
		reply: &WireReply<'_>,
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if self.finished {
			return Err(DecodeError::OutOfOrder("chunk after the end"));
		}
		self.start(reply, sink);
		self.usage(reply, sink)?;
		if let Some(error) = &reply.error {
			self.finished = true;
			sink(Event::Error(provider_error(error)));
			return Ok(());
		}
		let candidate = match reply.candidates.as_slice() {
			[] => None,
			[candidate] => Some(candidate),
			_ => return Err(DecodeError::UnsupportedItem("candidates".to_string())),
		};
		let Some(candidate) = candidate else {
			let feedback = reply.prompt_feedback.as_ref();
			if let Some(reason) = feedback.and_then(|feedback| feedback.block_reason.as_deref()) {
				self.end(
					Event::Finish(Outcome::Incomplete {
						reason: StopReason::ContentFilter,
						raw: Some(reason),
					}),
					sink,
				)?;
			}
			return Ok(());
		};
		let parts = candidate.content.iter().flat_map(|content| &content.parts);
		for raw in parts {
			self.part(raw, max_block_bytes, sink)?;
		}
		if let Some(reason) = candidate.finish_reason.as_deref() {
			self.end(ending(reason, candidate.finish_message.as_deref()), sink)?;
		}
		Ok(())
	}

	fn start(&mut self, reply: &WireReply<'_>, sink: &mut dyn FnMut(Event<'_>)) {
		if std::mem::replace(&mut self.started, true) {
			return;
		}
		self.id = reply
			.id
			.as_deref()
			.unwrap_or(&self.fallback_reply_id)
			.to_string();
		sink(Event::ResponseStart {
			id: &self.id,
			model: reply.model.as_deref().unwrap_or(&self.requested_model),
		});
	}

	/// Sent with every chunk that has any, because a stream can stop at any point
	/// and tokens are spent by then. A later chunk may repeat only some counts.
	fn usage(
		&mut self,
		reply: &WireReply<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let Some(usage) = &reply.usage_metadata else {
			return Ok(());
		};
		self.tokens.merge(usage);
		if let Some(tier) = usage.tier() {
			sink(Event::ServiceTier(tier));
		}
		sink(Event::Usage(self.tokens.usage()?));
		Ok(())
	}

	fn part(
		&mut self,
		raw: &RawValue,
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let part: WirePart<'_> = serde_json::from_str(raw.get())?;
		let signature = part.signature()?;
		if let Some(call) = &part.function_call {
			self.close(None, sink)?;
			if let Some(signature) = signature {
				self.signed_part(signature, raw, sink)?;
			}
			return self.call(call, sink);
		}
		let text = part
			.text
			.ok_or_else(|| DecodeError::UnsupportedItem("part".to_string()))?;
		let text = JsonStr::read(text, "text")?;
		let thought = part.thought == Some(true);
		if text.raw() != r#""""# {
			self.delta(thought, text, max_block_bytes, sink)?;
		}
		let thought_open = self
			.open
			.as_ref()
			.is_some_and(|open| open.thought.is_some());
		match signature {
			Some(signature) if thought && thought_open => self.close(Some(signature), sink),
			// Text, or a thought with no text of its own to sign.
			Some(signature) => {
				self.close(None, sink)?;
				self.signed_part(signature, raw, sink)
			},
			None => Ok(()),
		}
	}

	fn delta(
		&mut self,
		thought: bool,
		text: JsonStr<'_>,
		max_block_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		if self
			.open
			.as_ref()
			.is_some_and(|open| open.thought.is_some() != thought)
		{
			self.close(None, sink)?;
		}
		if self.open.is_none() {
			let part = self.parts.allocate()?;
			let kind = if thought {
				PartKind::Reasoning
			} else {
				PartKind::Text
			};
			sink(Event::PartStart { part, kind });
			self.open = Some(Open {
				part,
				thought: thought.then(OpenText::default),
				decoded_bytes: 0,
			});
		}
		let Some(open) = &mut self.open else {
			return Err(DecodeError::OutOfOrder("delta outside a part"));
		};
		open.decoded_bytes += text.decoded_len();
		if let Some(thought) = &mut open.thought {
			thought.push(&text.decode());
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

	/// A thought's signature, if it has one, stays in the item a later turn resends.
	fn close(
		&mut self,
		signature: Option<JsonStr<'_>>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let Some(open) = self.open.take() else {
			return Ok(());
		};
		let Some(thought) = open.thought else {
			sink(Event::PartEnd {
				part: open.part,
				value: Final::Text,
			});
			return Ok(());
		};
		let raw = thought.finish();
		let text = JsonStr::read(serde_json::from_str(&raw)?, "text")?;
		let item = thought_item(text, signature)?;
		sink(Event::PartEnd {
			part: open.part,
			value: Final::Reasoning {
				text: Some(text),
				encrypted: signature,
				item: &item,
			},
		});
		Ok(())
	}

	fn signed_part(
		&mut self,
		signature: JsonStr<'_>,
		item: &RawValue,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let part = self.parts.allocate()?;
		sink(Event::PartStart {
			part,
			kind: PartKind::Reasoning,
		});
		sink(Event::PartEnd {
			part,
			value: Final::Reasoning {
				text: None,
				encrypted: Some(signature),
				item,
			},
		});
		Ok(())
	}

	fn call(
		&mut self,
		call: &WireCall<'_>,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let name = call
			.name
			.as_deref()
			.ok_or(DecodeError::WrongType("functionCall"))?;
		let arguments = call.args.map_or("{}", RawValue::get);
		if !arguments.starts_with('{') {
			return Err(DecodeError::WrongType("args"));
		}
		let number = self.next_call;
		self.next_call += 1;
		let call_id = match call.id.as_deref() {
			Some(id) => Cow::Borrowed(id),
			None => Cow::Owned(format!("call_{}_{number}", self.id)),
		};
		let part = self.parts.allocate()?;
		sink(Event::PartStart {
			part,
			kind: PartKind::ToolCall {
				call_id: &call_id,
				name,
			},
		});
		let raw = string_value(arguments)?;
		let value = JsonStr::read(&raw, "args")?;
		sink(Event::PartEnd {
			part,
			value: Final::ToolCall(Arguments::complete(value).unwrap_or(Arguments::Partial(value))),
		});
		Ok(())
	}

	fn end(&mut self, ending: Event<'_>, sink: &mut dyn FnMut(Event<'_>)) -> Result<(), DecodeError> {
		self.close(None, sink)?;
		self.finished = true;
		sink(ending);
		Ok(())
	}
}

#[derive(Default)]
struct Tokens {
	prompt: Option<u64>,
	candidates: Option<u64>,
	total: Option<u64>,
	cached: Option<u64>,
	thoughts: Option<u64>,
}

impl Tokens {
	fn merge(&mut self, usage: &WireUsage<'_>) {
		self.prompt = usage.prompt_token_count.or(self.prompt);
		self.candidates = usage.candidates_token_count.or(self.candidates);
		self.total = usage.total_token_count.or(self.total);
		self.cached = usage.cached_content_token_count.or(self.cached);
		self.thoughts = usage.thoughts_token_count.or(self.thoughts);
	}

	/// `candidatesTokenCount` leaves thoughts out, and both clients count them as
	/// output, so they're added here.
	fn usage(&self) -> Result<Usage, DecodeError> {
		let output_tokens = match (self.candidates, self.thoughts) {
			(Some(answer), Some(thoughts)) => Some(
				answer
					.checked_add(thoughts)
					.ok_or(DecodeError::WrongType("usageMetadata"))?,
			),
			(answer, thoughts) => answer.or(thoughts),
		};
		Ok(Usage {
			input_tokens: self.prompt,
			output_tokens,
			total_tokens: self.total,
			cached_tokens: self.cached,
			cache_write_tokens: None,
			cache_write_12h_tokens: None,
			reasoning_tokens: self.thoughts,
		})
	}
}

/// Messages clients get text as it streams, but Gemini may sign that text in a
/// later chunk. Late text signatures are dropped; call signatures and signatures
/// with no preceding text are kept.
pub struct GeminiMessagesCompat<'a> {
	sequencer: Sequencer<'a>,
	/// A reasoning part after text. Its final value shows whether it signs text.
	waiting: Option<PartId>,
	text_sent: bool,
	dropped_text_signatures: usize,
}

impl<'a> GeminiMessagesCompat<'a> {
	/// `max_held_bytes` bounds parts waiting for Messages block order.
	#[must_use]
	pub fn new(scope: StateScope<'a>, max_held_bytes: usize) -> Self {
		Self {
			sequencer: Sequencer::new(scope, max_held_bytes),
			waiting: None,
			text_sent: false,
			dropped_text_signatures: 0,
		}
	}

	#[must_use]
	pub const fn dropped_text_signatures(&self) -> usize {
		self.dropped_text_signatures
	}

	#[must_use]
	pub const fn output_started(&self) -> bool {
		self.sequencer.output_started()
	}

	#[must_use]
	pub const fn is_ended(&self) -> bool {
		self.sequencer.is_ended()
	}

	pub fn event(&mut self, event: &Event<'_>, sink: &mut dyn Sink) {
		if let Some(part) = self.waiting.take() {
			if signs_text_before(event, part) {
				self.dropped_text_signatures += 1;
				return;
			}
			self.sequencer.event(
				&Event::PartStart {
					part,
					kind: PartKind::Reasoning,
				},
				sink,
			);
		}
		match *event {
			Event::PartStart {
				kind: PartKind::Text,
				..
			} => self.text_sent = true,
			Event::PartStart {
				part,
				kind: PartKind::Reasoning,
			} if self.text_sent => {
				self.waiting = Some(part);
				return;
			},
			_ => {},
		}
		self.sequencer.event(event, sink);
	}

	pub fn fail(&mut self, failure: &Failure, sink: &mut dyn Sink) {
		self.sequencer.fail(failure, sink);
	}
}

fn signs_text_before(event: &Event<'_>, waiting: PartId) -> bool {
	let Event::PartEnd {
		part,
		value: Final::Reasoning {
			text: None, item, ..
		},
	} = event
	else {
		return false;
	};
	*part == waiting && signs_text(item)
}

/// `GeminiMessagesCompat` moves these.
fn signs_text(item: &RawValue) -> bool {
	serde_json::from_str::<WirePart<'_>>(item.get())
		.is_ok_and(|part| part.function_call.is_none() && part.thought != Some(true))
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
	#[derive(Deserialize)]
	struct Wire<'a> {
		#[serde(borrow)]
		error: Error<'a>,
	}
	#[derive(Deserialize)]
	struct Error<'a> {
		#[serde(borrow, default)]
		details: Vec<Detail<'a>>,
	}
	#[derive(Deserialize)]
	#[serde(rename_all = "camelCase")]
	struct Detail<'a> {
		#[serde(borrow, default)]
		field_violations: Vec<Violation<'a>>,
	}
	#[derive(Deserialize)]
	struct Violation<'a> {
		#[serde(borrow)]
		field: Cow<'a, str>,
	}
	let wire: Wire<'_> = serde_json::from_slice(body).ok()?;
	let violation = wire
		.error
		.details
		.into_iter()
		.flat_map(|detail| detail.field_violations)
		.next()?;
	Some(violation.field.into_owned())
}

/// A whole reply, as the same events a stream gives.
pub fn decode_buffered(
	body: &[u8],
	requested_model: &str,
	fallback_reply_id: &str,
	sink: &mut dyn FnMut(Event<'_>),
) -> Result<(), DecodeError> {
	let reply: WireReply<'_> = serde_json::from_slice(body)?;
	let mut decoder = GeminiDecoder::new(requested_model, fallback_reply_id);
	decoder.reply(&reply, usize::MAX, sink)?;
	decoder.finish()
}

fn thought_item(
	text: JsonStr<'_>,
	signature: Option<JsonStr<'_>>,
) -> Result<Box<RawValue>, DecodeError> {
	let mut item = format!(r#"{{"text":{},"thought":true"#, text.raw());
	if let Some(signature) = signature {
		item.push_str(r#","thoughtSignature":"#);
		item.push_str(signature.raw());
	}
	item.push('}');
	Ok(RawValue::from_string(item)?)
}

/// An unknown reason fails, because it may mean the reply can't be trusted.
fn ending<'a>(reason: &'a str, message: Option<&'a str>) -> Event<'a> {
	let incomplete = |kind| {
		Event::Finish(Outcome::Incomplete {
			reason: kind,
			raw: Some(reason),
		})
	};
	let failed = |kind| {
		Event::Error(ProviderError {
			kind,
			code: Some(reason),
			message,
			client_message: None,
		})
	};
	match reason {
		"STOP" => Event::Finish(Outcome::Completed),
		"MAX_TOKENS" | "CONTINUATION" => incomplete(StopReason::MaxTokens),
		"SAFETY"
		| "RECITATION"
		| "LANGUAGE"
		| "BLOCKLIST"
		| "PROHIBITED_CONTENT"
		| "SPII"
		| "IMAGE_SAFETY"
		| "IMAGE_PROHIBITED_CONTENT"
		| "IMAGE_RECITATION"
		| "MODEL_ARMOR"
		| "ESCALATION" => incomplete(StopReason::ContentFilter),
		"MISSING_THOUGHT_SIGNATURE" => failed(ErrorKind::InvalidRequest),
		"PUP_LIMITED_DISABLED" => failed(ErrorKind::Permission),
		_ => failed(ErrorKind::Unknown),
	}
}

fn provider_error<'a>(error: &'a WireError<'a>) -> ProviderError<'a> {
	ProviderError {
		kind: error_kind(error),
		code: error.status.as_deref(),
		message: error.message.as_deref(),
		client_message: None,
	}
}

/// Gemini has no code for a prompt that's too long, so only its wording shows it.
/// A bad API key is `INVALID_ARGUMENT` too, told apart only by its detail's reason.
fn error_kind(error: &WireError<'_>) -> ErrorKind {
	let message = error.message.as_deref().unwrap_or_default();
	let bad_key = error
		.details
		.iter()
		.any(|detail| detail.reason.as_deref() == Some("API_KEY_INVALID"));
	match error.status.as_deref() {
		Some("INVALID_ARGUMENT") if bad_key => ErrorKind::Authentication,
		Some("INVALID_ARGUMENT")
			if error.code == Some(400)
				&& message.contains("input token count")
				&& message.contains("exceeds the maximum number of tokens allowed") =>
		{
			ErrorKind::ContextTooLong
		},
		Some("INVALID_ARGUMENT" | "FAILED_PRECONDITION") => ErrorKind::InvalidRequest,
		Some("UNAUTHENTICATED") => ErrorKind::Authentication,
		Some("PERMISSION_DENIED") => ErrorKind::Permission,
		Some("NOT_FOUND") => ErrorKind::NotFound,
		// Spent prepaid credit shares this status. Retrying can't help, a rate limit can.
		Some("RESOURCE_EXHAUSTED") if error.code == Some(402) => ErrorKind::Billing,
		Some("RESOURCE_EXHAUSTED") => ErrorKind::RateLimit,
		Some("UNAVAILABLE") => ErrorKind::Overloaded,
		_ => ErrorKind::Unknown,
	}
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireReply<'a> {
	#[serde(default, borrow)]
	candidates: Vec<WireCandidate<'a>>,
	#[serde(borrow)]
	prompt_feedback: Option<WireFeedback<'a>>,
	#[serde(borrow)]
	usage_metadata: Option<WireUsage<'a>>,
	#[serde(rename = "modelVersion", borrow)]
	model: Option<Cow<'a, str>>,
	#[serde(rename = "responseId", borrow)]
	id: Option<Cow<'a, str>>,
	#[serde(borrow)]
	error: Option<WireError<'a>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireCandidate<'a> {
	#[serde(borrow)]
	content: Option<WireContent<'a>>,
	#[serde(borrow)]
	finish_reason: Option<Cow<'a, str>>,
	#[serde(borrow)]
	finish_message: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct WireContent<'a> {
	#[serde(default, borrow)]
	parts: Vec<&'a RawValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireFeedback<'a> {
	#[serde(borrow)]
	block_reason: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WirePart<'a> {
	#[serde(borrow)]
	text: Option<&'a RawValue>,
	thought: Option<bool>,
	#[serde(alias = "thought_signature", borrow)]
	thought_signature: Option<&'a RawValue>,
	#[serde(borrow)]
	function_call: Option<WireCall<'a>>,
}

impl<'a> WirePart<'a> {
	fn signature(&self) -> Result<Option<JsonStr<'a>>, DecodeError> {
		let Some(raw) = self.thought_signature else {
			return Ok(None);
		};
		let signature = JsonStr::read(raw, "thoughtSignature")?;
		Ok((signature.raw() != r#""""#).then_some(signature))
	}
}

#[derive(Deserialize)]
struct WireCall<'a> {
	#[serde(borrow)]
	id: Option<Cow<'a, str>>,
	#[serde(borrow)]
	name: Option<Cow<'a, str>>,
	#[serde(borrow)]
	args: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct WireError<'a> {
	code: Option<u64>,
	#[serde(borrow)]
	message: Option<Cow<'a, str>>,
	#[serde(borrow)]
	status: Option<Cow<'a, str>>,
	#[serde(default, borrow)]
	details: Vec<WireErrorDetail<'a>>,
}

#[derive(Deserialize)]
struct WireErrorDetail<'a> {
	#[serde(borrow)]
	reason: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireUsage<'a> {
	prompt_token_count: Option<u64>,
	candidates_token_count: Option<u64>,
	total_token_count: Option<u64>,
	cached_content_token_count: Option<u64>,
	thoughts_token_count: Option<u64>,
	#[serde(borrow)]
	service_tier: Option<Cow<'a, str>>,
	#[serde(borrow)]
	traffic_type: Option<Cow<'a, str>>,
}

impl WireUsage<'_> {
	fn tier(&self) -> Option<&'static str> {
		let tier = self
			.service_tier
			.as_deref()
			.or(self.traffic_type.as_deref());
		match tier? {
			"standard" | "ON_DEMAND" => Some("default"),
			"flex" | "ON_DEMAND_FLEX" => Some("flex"),
			"priority" | "ON_DEMAND_PRIORITY" => Some("priority"),
			"PROVISIONED_THROUGHPUT" => Some("scale"),
			_ => None,
		}
	}
}

#[cfg(test)]
#[path = "decoder_tests.rs"]
mod tests;
