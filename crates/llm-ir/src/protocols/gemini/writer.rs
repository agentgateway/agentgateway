//! Replies for a Gemini client: one `GenerateContentResponse`, or a stream of them
//! with `alt=sse`, one per event.
//!
//! Nothing goes out until the first part, so an early failure can still be an HTTP
//! error; a later one ends the stream with an error event. Only the first open part
//! streams, and later ones wait. A function call goes out whole, at its final value.
//!
//! Reasoning goes back to the client encoded in the `thoughtSignature` of the next
//! part that isn't a thought, because Gemini CLI drops thought parts from its
//! history. When nothing follows, an empty text part contains it.

use std::collections::VecDeque;
use std::mem;

use super::write_signature;
use crate::carrier::{self, StateScope};
use crate::ir::events::{
	Arguments, ErrorKind, Event, Final, JsonStr, Outcome, PartId, PartKind, StopReason, Usage,
	UsageFold,
};
use crate::json::push_string;
use crate::prompt::RequestRefusal;
pub use crate::protocols::responses::writer::Progress;
use crate::protocols::{HttpReply, is_field_name};

pub struct GeminiStream<'a> {
	state: State<'a>,
	out: Vec<u8>,
	started: bool,
	early_failure: Option<GeminiError>,
}

impl<'a> GeminiStream<'a> {
	/// `max_held_bytes` bounds the text kept for parts waiting their turn.
	#[must_use]
	pub fn new(scope: StateScope<'a>, max_held_bytes: usize) -> Self {
		Self {
			state: State::new(scope, max_held_bytes, false),
			out: Vec::new(),
			started: false,
			early_failure: None,
		}
	}

	/// Only for a client that asked with `includeThoughts`.
	#[must_use]
	pub const fn show_thoughts(mut self, show: bool) -> Self {
		self.state.show_thoughts = show;
		self
	}

	pub fn event(&mut self, event: &Event<'_>) {
		self.state.event(event);
		self.write();
	}

	/// Later events are ignored.
	pub fn fail(&mut self, status: u16) {
		self.state.fail(classify(ErrorKind::Unknown, Some(status)));
		self.write();
	}

	/// `HasOutput` comes first: an ended reply isn't finished until it's drained.
	#[must_use]
	pub const fn progress(&self) -> Progress {
		if !self.out.is_empty() {
			Progress::HasOutput
		} else if self.state.ended {
			Progress::Finished
		} else {
			Progress::NeedsInput
		}
	}

	pub fn take_stream(&mut self) -> Vec<u8> {
		mem::take(&mut self.out)
	}

	/// The HTTP reply for a failure before any output. When it's `Some`, nothing
	/// was written, and the caller sends this instead of a stream.
	pub fn early_failure(&self) -> Option<HttpReply> {
		self.early_failure.as_ref().map(GeminiError::reply)
	}

	fn write(&mut self) {
		let parts = mem::take(&mut self.state.parts_out);
		let citations = mem::take(&mut self.state.citations_out);
		let ending = self.state.ending.take();
		let finish = match &ending {
			Some(Ending::Finish(reason)) => Some(*reason),
			_ => None,
		};
		if !parts.is_empty() || !citations.is_empty() || finish.is_some() {
			let mut json = String::new();
			self
				.state
				.write_response(&parts, &citations, finish, &mut json);
			self.out.extend_from_slice(b"data: ");
			self.out.extend_from_slice(json.as_bytes());
			self.out.extend_from_slice(b"\r\n\r\n");
			self.started = true;
		}
		if let Some(Ending::Failure(error)) = ending {
			if self.started {
				self.out.extend_from_slice(b"data: ");
				self.out.extend_from_slice(&error.body());
				self.out.extend_from_slice(b"\r\n\r\n");
			} else {
				self.early_failure = Some(error);
			}
		}
	}
}

pub struct GeminiBuffered<'a> {
	state: State<'a>,
	parts: Vec<OutPart>,
	citations: Vec<String>,
	ending: Option<Ending>,
}

impl<'a> GeminiBuffered<'a> {
	/// `max_output_bytes` bounds the reply's output.
	#[must_use]
	pub fn new(scope: StateScope<'a>, max_output_bytes: usize) -> Self {
		Self {
			state: State::new(scope, max_output_bytes, true),
			parts: Vec::new(),
			citations: Vec::new(),
			ending: None,
		}
	}

	/// Only for a client that asked with `includeThoughts`.
	#[must_use]
	pub const fn show_thoughts(mut self, show: bool) -> Self {
		self.state.show_thoughts = show;
		self
	}

	pub fn event(&mut self, event: &Event<'_>) {
		self.state.event(event);
		self.collect();
	}

	/// Later events are ignored.
	pub fn fail(&mut self, status: u16) {
		self.state.fail(classify(ErrorKind::Unknown, Some(status)));
		self.collect();
	}

	/// A reply that never ended is a 500, so a caller whose input stopped early
	/// should say why with `fail`.
	#[must_use]
	pub fn into_reply(self) -> HttpReply {
		match self.ending {
			Some(Ending::Finish(reason)) => {
				let mut body = String::new();
				self
					.state
					.write_response(&self.parts, &self.citations, Some(reason), &mut body);
				HttpReply {
					status: 200,
					body: body.into_bytes(),
				}
			},
			Some(Ending::Failure(error)) => error.reply(),
			None => classify(ErrorKind::Unknown, Some(500)).reply(),
		}
	}

	/// Text from one part is joined into one Gemini part, as a reply that isn't
	/// streamed has it.
	fn collect(&mut self) {
		for part in mem::take(&mut self.state.parts_out) {
			match (self.parts.last_mut(), part) {
				(
					Some(OutPart::Text {
						body: last,
						thought: last_thought,
						..
					}),
					OutPart::Text {
						body,
						thought,
						signature: None,
					},
				) if *last_thought == thought => last.push_str(&body),
				(_, part) => self.parts.push(part),
			}
		}
		self
			.citations
			.extend(mem::take(&mut self.state.citations_out));
		if let Some(ending) = self.state.ending.take() {
			self.ending = Some(ending);
		}
	}
}

/// What a Gemini client is told, in Google's words. The text is fixed: a provider's
/// words can describe the gateway's account, not the client's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeminiError {
	pub status: Status,
	pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
	InvalidArgument,
	FailedPrecondition,
	Unauthenticated,
	PermissionDenied,
	NotFound,
	ResourceExhausted,
	Unavailable,
	Internal,
}

impl Status {
	#[must_use]
	pub const fn code(self) -> u16 {
		match self {
			Self::InvalidArgument | Self::FailedPrecondition => 400,
			Self::Unauthenticated => 401,
			Self::PermissionDenied => 403,
			Self::NotFound => 404,
			Self::ResourceExhausted => 429,
			Self::Unavailable => 503,
			Self::Internal => 500,
		}
	}

	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::InvalidArgument => "INVALID_ARGUMENT",
			Self::FailedPrecondition => "FAILED_PRECONDITION",
			Self::Unauthenticated => "UNAUTHENTICATED",
			Self::PermissionDenied => "PERMISSION_DENIED",
			Self::NotFound => "NOT_FOUND",
			Self::ResourceExhausted => "RESOURCE_EXHAUSTED",
			Self::Unavailable => "UNAVAILABLE",
			Self::Internal => "INTERNAL",
		}
	}

	const fn message(self) -> &'static str {
		match self {
			Self::InvalidArgument => "The provider refused the request.",
			Self::FailedPrecondition => "The provider's account can't serve this request.",
			Self::Unauthenticated => "The provider refused the gateway's credentials.",
			Self::PermissionDenied => "The provider refused access.",
			Self::NotFound => "The provider has no such model or resource.",
			Self::ResourceExhausted => "The provider's rate limit was reached.",
			Self::Unavailable => "The provider is overloaded.",
			Self::Internal => "The provider's reply could not be served.",
		}
	}
}

impl GeminiError {
	#[must_use]
	pub fn reply(&self) -> HttpReply {
		HttpReply {
			status: self.status.code(),
			body: self.body(),
		}
	}

	#[must_use]
	pub fn body(&self) -> Vec<u8> {
		let mut out = format!(r#"{{"error":{{"code":{},"message":"#, self.status.code());
		push_string(&mut out, &self.message);
		out.push_str(r#","status":""#);
		out.push_str(self.status.as_str());
		out.push_str(r#""}}"#);
		out.into_bytes()
	}
}

#[must_use]
pub fn classify(kind: ErrorKind, status: Option<u16>) -> GeminiError {
	let status = match kind {
		ErrorKind::ContextTooLong => {
			return GeminiError {
				status: Status::InvalidArgument,
				message: "The input is too long for the model.".into(),
			};
		},
		ErrorKind::InvalidRequest | ErrorKind::RequestTooLarge => Status::InvalidArgument,
		ErrorKind::Authentication => Status::Unauthenticated,
		ErrorKind::Permission => Status::PermissionDenied,
		ErrorKind::NotFound => Status::NotFound,
		ErrorKind::Billing => Status::FailedPrecondition,
		ErrorKind::RateLimit => Status::ResourceExhausted,
		ErrorKind::Overloaded => Status::Unavailable,
		ErrorKind::Unknown => match status {
			Some(400 | 413 | 422) => Status::InvalidArgument,
			Some(401) => Status::Unauthenticated,
			Some(403) => Status::PermissionDenied,
			Some(404) => Status::NotFound,
			Some(429) => Status::ResourceExhausted,
			Some(503 | 529) => Status::Unavailable,
			_ => Status::Internal,
		},
	};
	GeminiError {
		status,
		message: status.message().into(),
	}
}

/// A provider's HTTP error as a Gemini client's. A field name is the client's own
/// vocabulary, so it's safe to pass on where the provider's words aren't.
#[must_use]
pub fn error_reply_naming(status: u16, kind: ErrorKind, field: Option<&str>) -> HttpReply {
	let mut error = classify(kind, Some(status));
	if error.status == Status::InvalidArgument
		&& error.message == Status::InvalidArgument.message()
		&& let Some(field) = field.filter(|field| is_field_name(field))
	{
		error.message = format!("The provider refused `{field}`.");
	}
	error.reply()
}

/// A request the core refused, as Gemini's 400.
#[must_use]
pub fn refusal_reply(refusal: &RequestRefusal) -> HttpReply {
	GeminiError {
		status: Status::InvalidArgument,
		message: refusal.to_string(),
	}
	.reply()
}

enum Ending {
	Finish(&'static str),
	Failure(GeminiError),
}

/// One Gemini part, ready to write. `body` is a JSON string's text between its
/// quotes, so text from deltas joins without decoding.
enum OutPart {
	Text {
		body: String,
		thought: bool,
		signature: Option<String>,
	},
	Call {
		id: String,
		name: String,
		arguments: String,
		signature: Option<String>,
	},
}

struct State<'a> {
	scope: StateScope<'a>,
	max_bytes: usize,
	/// A buffered reply keeps every part, so they count toward `max_bytes`.
	keeps_output: bool,
	show_thoughts: bool,
	id: String,
	model: String,
	/// Parts not fully written, in start order. Only the first streams.
	open: VecDeque<OpenPart>,
	bytes: usize,
	/// Carriers for the next part that isn't a thought.
	pending: Vec<String>,
	dropped_call: bool,
	usage: UsageFold,
	parts_out: Vec<OutPart>,
	citations_out: Vec<String>,
	ending: Option<Ending>,
	ended: bool,
}

struct OpenPart {
	id: PartId,
	kind: Kind,
	/// Text from deltas that came while another part was first.
	held: String,
	streamed: bool,
	done: Option<Done>,
}

enum Kind {
	Text,
	Thought,
	Call { id: String, name: String },
}

enum Done {
	Text,
	Refusal(String),
	Call(Option<String>),
	Reasoning { text: Option<String>, item: String },
}

impl<'a> State<'a> {
	fn new(scope: StateScope<'a>, max_bytes: usize, keeps_output: bool) -> Self {
		Self {
			scope,
			max_bytes,
			keeps_output,
			show_thoughts: false,
			id: String::new(),
			model: scope.model.to_string(),
			open: VecDeque::new(),
			bytes: 0,
			pending: Vec::new(),
			dropped_call: false,
			usage: UsageFold::default(),
			parts_out: Vec::new(),
			citations_out: Vec::new(),
			ending: None,
			ended: false,
		}
	}

	fn event(&mut self, event: &Event<'_>) {
		if self.ended {
			return;
		}
		match *event {
			Event::ResponseStart { id, model } => {
				self.id = id.to_string();
				self.model = model.to_string();
			},
			Event::PartStart { part, kind } => {
				let kind = match kind {
					PartKind::Text | PartKind::Refusal => Kind::Text,
					PartKind::Reasoning => Kind::Thought,
					PartKind::ToolCall { call_id, name } => Kind::Call {
						id: call_id.to_string(),
						name: name.to_string(),
					},
				};
				self.open.push_back(OpenPart {
					id: part,
					kind,
					held: String::new(),
					streamed: false,
					done: None,
				});
			},
			Event::Delta { part, text } => self.delta(part, text),
			Event::PartEnd { part, value } => {
				let Some(open) = self.open.iter_mut().find(|open| open.id == part) else {
					return;
				};
				open.done = Some(Done::from(value));
				self.advance();
			},
			Event::Citation { url, .. } => {
				let mut source = String::from(r#"{"uri":"#);
				push_string(&mut source, url);
				source.push('}');
				if self.count(source.len()) {
					self.citations_out.push(source);
				}
			},
			Event::Usage(_) => self.usage.event(event),
			Event::Finish(outcome) => self.finish(outcome),
			Event::Error(error) => {
				let mut failure = classify(error.kind, None);
				if let Some(message) = error.client_message {
					failure.message = message.to_string();
				}
				self.fail(failure);
			},
			Event::MessageEnd { .. } | Event::ServiceTier(_) | Event::StopSequence(_) => {},
		}
	}

	fn delta(&mut self, part: PartId, text: JsonStr<'_>) {
		let Some(at) = self.open.iter().position(|open| open.id == part) else {
			return;
		};
		let thought = match self.open[at].kind {
			Kind::Call { .. } => return,
			Kind::Thought if !self.show_thoughts => return,
			Kind::Thought => true,
			Kind::Text => false,
		};
		let raw = text.raw();
		let body = &raw[1..raw.len() - 1];
		if at == 0 {
			self.open[at].streamed = true;
			self.text(body.to_string(), thought);
		} else if self.count(body.len()) {
			self.open[at].held.push_str(body);
		}
	}

	/// The first part streams its held text when it becomes first, and goes out
	/// once it has ended.
	fn advance(&mut self) {
		while let Some(front) = self.open.front_mut() {
			if !front.held.is_empty() {
				let held = mem::take(&mut front.held);
				front.streamed = true;
				let thought = matches!(front.kind, Kind::Thought);
				self.bytes -= held.len();
				self.text(held, thought);
				continue;
			}
			if front.done.is_none() {
				return;
			}
			let Some(part) = self.open.pop_front() else {
				return;
			};
			self.close(part);
			if self.ended {
				return;
			}
		}
	}

	fn close(&mut self, part: OpenPart) {
		let Some(done) = part.done else {
			return;
		};
		match (part.kind, done) {
			(Kind::Text, Done::Text) => {},
			(Kind::Text, Done::Refusal(body)) => {
				if !part.streamed {
					self.text(body, false);
				}
			},
			(Kind::Thought, Done::Reasoning { text, item }) => {
				if self.show_thoughts
					&& !part.streamed
					&& let Some(text) = text
				{
					self.text(text, true);
				}
				let carrier = carrier::encode(
					self.scope,
					carrier::CarrierPurpose::GeminiThoughtSignature,
					item.as_bytes(),
				);
				if self.count(carrier.len()) {
					self.pending.push(carrier);
				}
			},
			(Kind::Call { id, name }, Done::Call(Some(arguments))) => {
				let signature = self.signature();
				self.out(OutPart::Call {
					id,
					name,
					arguments,
					signature,
				});
			},
			(Kind::Call { .. }, Done::Call(None)) => self.dropped_call = true,
			_ => self.fail(classify(ErrorKind::Unknown, Some(500))),
		}
	}

	fn text(&mut self, body: String, thought: bool) {
		let signature = if thought { None } else { self.signature() };
		self.out(OutPart::Text {
			body,
			thought,
			signature,
		});
	}

	fn signature(&mut self) -> Option<String> {
		(!self.pending.is_empty()).then(|| {
			let carriers = mem::take(&mut self.pending);
			write_signature(&carriers)
		})
	}

	fn out(&mut self, part: OutPart) {
		if self.keeps_output {
			let size = match &part {
				OutPart::Text { body, .. } => body.len(),
				OutPart::Call { arguments, .. } => arguments.len(),
			};
			if !self.count(size) {
				return;
			}
		}
		self.parts_out.push(part);
	}

	/// Whether `bytes` more fit. A reply over the limit fails.
	fn count(&mut self, bytes: usize) -> bool {
		self.bytes += bytes;
		if self.bytes > self.max_bytes {
			self.fail(classify(ErrorKind::Unknown, Some(500)));
			return false;
		}
		true
	}

	fn finish(&mut self, outcome: Outcome<'_>) {
		if !self.open.is_empty() {
			self.fail(classify(ErrorKind::Unknown, Some(500)));
			return;
		}
		let reason = match outcome {
			Outcome::Completed if self.dropped_call => {
				self.fail(classify(ErrorKind::Unknown, Some(500)));
				return;
			},
			Outcome::Completed => "STOP",
			Outcome::Incomplete { reason, .. } => match reason {
				StopReason::MaxTokens => "MAX_TOKENS",
				StopReason::ContentFilter => "SAFETY",
				StopReason::ContextWindow => {
					self.fail(classify(ErrorKind::ContextTooLong, None));
					return;
				},
				StopReason::Other => "OTHER",
			},
		};
		if !self.pending.is_empty() {
			self.text(String::new(), false);
		}
		self.ending = Some(Ending::Finish(reason));
		self.ended = true;
	}

	fn fail(&mut self, error: GeminiError) {
		if self.ended {
			return;
		}
		self.ended = true;
		self.open.clear();
		self.pending.clear();
		self.ending = Some(Ending::Failure(error));
	}

	fn write_response(
		&self,
		parts: &[OutPart],
		citations: &[String],
		finish: Option<&str>,
		out: &mut String,
	) {
		out.push_str(r#"{"candidates":[{"#);
		if !parts.is_empty() {
			out.push_str(r#""content":{"role":"model","parts":["#);
			for (at, part) in parts.iter().enumerate() {
				if at > 0 {
					out.push(',');
				}
				write_part(part, out);
			}
			out.push_str("]},");
		}
		if let Some(reason) = finish {
			out.push_str(r#""finishReason":""#);
			out.push_str(reason);
			out.push_str(r#"","#);
		}
		if !citations.is_empty() {
			out.push_str(r#""citationMetadata":{"citationSources":["#);
			out.push_str(&citations.join(","));
			out.push_str("]},");
		}
		out.push_str(r#""index":0}]"#);
		if finish.is_some() {
			write_usage(self.usage.total(), out);
		}
		out.push_str(r#","modelVersion":"#);
		push_string(out, &self.model);
		out.push_str(r#","responseId":"#);
		push_string(out, &self.id);
		out.push('}');
	}
}

impl From<Final<'_>> for Done {
	fn from(value: Final<'_>) -> Self {
		let body = |text: JsonStr<'_>| {
			let raw = text.raw();
			raw[1..raw.len() - 1].to_string()
		};
		match value {
			Final::Text => Self::Text,
			Final::Refusal(text) => Self::Refusal(body(text)),
			Final::ToolCall(Arguments::Complete(arguments)) => {
				let text = Arguments::Complete(arguments).raw();
				// Complete arguments are strict JSON, so an object starts with a brace.
				match serde_json::from_str::<String>(text) {
					Ok(decoded) if decoded.is_empty() => Self::Call(Some("{}".to_string())),
					Ok(decoded) if decoded.starts_with('{') => Self::Call(Some(decoded)),
					_ => Self::Call(None),
				}
			},
			Final::ToolCall(Arguments::Partial(_)) => Self::Call(None),
			Final::Reasoning { text, item, .. } => Self::Reasoning {
				text: text.map(body),
				item: item.get().to_string(),
			},
		}
	}
}

fn write_part(part: &OutPart, out: &mut String) {
	let signature = match part {
		OutPart::Text {
			body,
			thought,
			signature,
		} => {
			out.push_str(r#"{"text":""#);
			out.push_str(body);
			out.push('"');
			if *thought {
				out.push_str(r#","thought":true"#);
			}
			signature
		},
		OutPart::Call {
			id,
			name,
			arguments,
			signature,
		} => {
			out.push_str(r#"{"functionCall":{"id":"#);
			push_string(out, id);
			out.push_str(r#","name":"#);
			push_string(out, name);
			out.push_str(r#","args":"#);
			out.push_str(arguments);
			out.push('}');
			signature
		},
	};
	if let Some(signature) = signature {
		out.push_str(r#","thoughtSignature":"#);
		push_string(out, signature);
	}
	out.push('}');
}

/// Gemini counts cached tokens inside `promptTokenCount`, as the core does, and
/// thoughts apart from the answer.
fn write_usage(usage: Usage, out: &mut String) {
	let thoughts = usage.reasoning_tokens;
	let answer = usage
		.output_tokens
		.map(|output| output.saturating_sub(thoughts.unwrap_or(0)));
	let total = usage.total_tokens.or_else(|| {
		usage
			.input_tokens
			.zip(usage.output_tokens)
			.map(|(input, output)| input.saturating_add(output))
	});
	let fields = [
		("promptTokenCount", usage.input_tokens),
		("cachedContentTokenCount", usage.cached_tokens),
		("candidatesTokenCount", answer),
		("thoughtsTokenCount", thoughts),
		("totalTokenCount", total),
	];
	out.push_str(r#","usageMetadata":{"#);
	let mut first = true;
	for (name, value) in fields {
		let Some(value) = value else {
			continue;
		};
		if !first {
			out.push(',');
		}
		first = false;
		out.push('"');
		out.push_str(name);
		out.push_str(r#"":"#);
		out.push_str(&value.to_string());
	}
	out.push('}');
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
