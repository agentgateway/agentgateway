#![doc = include_str!("README.md")]

use crate::carrier::StateScope;
use crate::ir::events::{DecodeError, Event, Telemetry, Usage, UsageFold};
use crate::protocols::chat_completions::decoder::CompletionsDecoder;
use crate::protocols::chat_completions::writer::{BufferedChat, ChatStream};
use crate::protocols::gemini::decoder::{GeminiDecoder, GeminiMessagesCompat};
use crate::protocols::gemini::writer::{GeminiBuffered, GeminiStream};
use crate::protocols::messages::decoder::MessagesDecoder;
use crate::protocols::messages::writer::{ReplyWriter, Sequencer, Sink, SseWriter};
use crate::protocols::messages::{ErrorType, Failure};
use crate::protocols::responses::decoder::ResponsesDecoder;
use crate::protocols::responses::writer::{
	BufferedResponse, Progress as WriterProgress, ResponseStream,
};
use crate::protocols::{
	ClientProtocol, HttpReply, ToolRoute, UpstreamProtocol, chat_completions, gemini, messages,
	responses,
};
use crate::reply::check::{Check, Checks, OutputChecks, Verdict};
use crate::reply::rewrite::ReplyRewrites;
use crate::reply::stop::StopAt;
use crate::sse::{Frame, SseError, SseParser};

pub mod check;
pub mod rewrite;
pub mod stop;

const READ_FAILED: &str = "The provider's reply could not be read.";

/// Protocols and request state needed to convert one provider reply.
pub struct ReplyContext<'a> {
	pub client: ClientProtocol,
	pub upstream: UpstreamProtocol,
	/// The model is the one sent upstream; a reply that names none takes it.
	pub scope: StateScope<'a>,
	/// [`crate::ReplyPlan::routes`].
	pub routes: &'a [ToolRoute],
	/// [`crate::ReplyPlan::show_reasoning`].
	pub show_reasoning: bool,
	/// [`crate::ReplyPlan::include_usage`].
	pub include_usage: bool,
	/// The client's Unix time, where its protocol has one.
	pub created: u64,
	/// Names a reply whose provider sent no id. Unique per attempt: call ids made up from it
	/// must not repeat in the conversation the client sends back.
	pub fallback_reply_id: &'a str,
	/// [`crate::ReplyPlan::emulated_stop_sequences`].
	pub stop_sequences: Vec<String>,
	/// Bounds what each writer holds, and each part a decoder builds.
	pub max_held_bytes: usize,
}

/// A provider's HTTP error, as the client's protocol says it. The status stays the
/// provider's, so a client's retry logic sees what the provider said.
#[must_use]
pub fn error_reply(
	client: ClientProtocol,
	upstream: UpstreamProtocol,
	status: u16,
	body: &[u8],
) -> HttpReply {
	let kind = match upstream {
		UpstreamProtocol::Messages => messages::decoder::http_error_kind(body),
		UpstreamProtocol::Responses { .. } => responses::decoder::http_error_kind(body),
		UpstreamProtocol::ChatCompletions => chat_completions::decoder::http_error_kind(body),
		UpstreamProtocol::Gemini => gemini::decoder::http_error_kind(body),
	};
	match client {
		ClientProtocol::Messages => {
			let field = match upstream {
				UpstreamProtocol::Messages => None,
				UpstreamProtocol::Responses { .. } => responses::decoder::http_error_field(body),
				UpstreamProtocol::ChatCompletions => chat_completions::decoder::http_error_field(body),
				UpstreamProtocol::Gemini => gemini::decoder::http_error_field(body),
			};
			let failure = messages::classify_refused(kind, Some(status), field.as_deref());
			HttpReply {
				status,
				body: messages::error_body(&failure),
			}
		},
		ClientProtocol::Responses => responses::writer::error_reply(status, kind),
		ClientProtocol::ChatCompletions => chat_completions::writer::error_reply(status, kind),
		ClientProtocol::Gemini => gemini::writer::error_reply_naming(status, kind, None),
	}
}

/// Converts one complete provider JSON body into a client reply.
pub struct BufferedReply<'a> {
	conversion: Conversion<BufferedWriter<'a>>,
	upstream: UpstreamProtocol,
	model: &'a str,
	fallback_reply_id: &'a str,
	read: bool,
}

impl<'a> BufferedReply<'a> {
	#[must_use]
	pub fn new(pair: ReplyContext<'a>) -> Self {
		let writer = BufferedWriter::new(&pair);
		Self {
			conversion: Conversion::new(pair.stop_sequences, writer),
			upstream: pair.upstream,
			model: pair.scope.model,
			fallback_reply_id: pair.fallback_reply_id,
			read: false,
		}
	}

	/// See [`ReplyRewrites`] for what installing one costs.
	pub fn set_rewrites(&mut self, rewrites: ReplyRewrites) {
		self.conversion.rewrites = rewrites;
	}

	/// Holds output back for the caller to check, after any rewrites.
	pub fn set_checks(&mut self, checks: OutputChecks) {
		self.conversion.checks = Some(Checks::new(checks));
	}

	/// Decodes one complete provider body. Errors also mark the client reply as failed.
	/// The caller must bound the body size.
	///
	/// # Panics
	///
	/// Panics if a body was already read.
	pub fn read(&mut self, body: &[u8]) -> Result<(), DecodeError> {
		assert!(!self.read, "a buffered reply is read once");
		self.read = true;
		let conversion = &mut self.conversion;
		let sink: &mut dyn FnMut(Event<'_>) = &mut |event| conversion.event(event);
		let result = match self.upstream {
			UpstreamProtocol::Messages => messages::decoder::decode_buffered(body, sink),
			UpstreamProtocol::Responses { .. } => responses::decoder::decode_buffered(body, sink),
			UpstreamProtocol::ChatCompletions => {
				chat_completions::decoder::decode_buffered(body, self.model, self.fallback_reply_id, sink)
			},
			UpstreamProtocol::Gemini => {
				gemini::decoder::decode_buffered(body, self.model, self.fallback_reply_id, sink)
			},
		};
		if result.is_err() {
			self.conversion.fail();
		}
		result
	}

	#[must_use]
	pub const fn usage(&self) -> Usage {
		self.conversion.usage.total()
	}

	#[must_use]
	pub const fn telemetry(&self) -> &Telemetry {
		&self.conversion.telemetry
	}

	/// Whether output waits for the caller's check before the reply can be written.
	pub fn progress(&mut self) -> ReplyProgress<'_> {
		ReplyProgress::of(&mut self.conversion)
	}

	/// Returns the client reply, including any recorded decoding failure.
	///
	/// # Panics
	///
	/// Panics if no body was read or a policy verdict is still pending.
	#[must_use]
	pub fn into_reply(self) -> HttpReply {
		assert!(self.read, "a buffered reply is read before writing it");
		assert!(
			!self.conversion.is_held(),
			"a held reply is resolved before it is written"
		);
		self.conversion.writer.into_reply()
	}
}

/// Converts provider SSE chunks into the client's stream.
pub struct StreamReply<'a> {
	conversion: Conversion<StreamWriter<'a>>,
	decoder: Decoder,
	sse: SseParser,
	max_event_bytes: usize,
	max_held_bytes: usize,
}

/// Framing or decoding failure while finishing a provider stream.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
	#[error(transparent)]
	Framing(#[from] SseError),
	#[error(transparent)]
	Decode(#[from] DecodeError),
}

impl<'a> StreamReply<'a> {
	/// `max_event_bytes` bounds one SSE event. Only a Responses provider's final
	/// event, which repeats the whole reply, is read in pieces past it.
	#[must_use]
	pub fn new(pair: ReplyContext<'a>, max_event_bytes: usize) -> Self {
		let writer = StreamWriter::new(&pair);
		Self {
			decoder: Decoder::new(pair.upstream, pair.scope.model, pair.fallback_reply_id),
			conversion: Conversion::new(pair.stop_sequences, writer),
			sse: SseParser::new(max_event_bytes),
			max_event_bytes,
			max_held_bytes: pair.max_held_bytes,
		}
	}

	/// See [`ReplyRewrites`] for what installing one costs.
	pub fn set_rewrites(&mut self, rewrites: ReplyRewrites) {
		self.conversion.rewrites = rewrites;
	}

	/// Holds output back for the caller to check, after any rewrites.
	pub fn set_checks(&mut self, checks: OutputChecks) {
		self.conversion.checks = Some(Checks::new(checks));
	}

	/// Feed one transport chunk. Returns the first decode error in the chunk after
	/// processing its remaining events for usage. Continue feeding until finished
	/// or the provider connection ends, even after an error.
	pub fn push(&mut self, chunk: &[u8]) -> Result<(), DecodeError> {
		if self.is_finished() {
			return Ok(());
		}
		self.sse.push(chunk);
		let mut error = None;
		while !self.decoder.is_finished()
			&& let Some(frame) = self.sse.next_frame()
		{
			let conversion = &mut self.conversion;
			let pushed = self.decoder.push(
				frame,
				self.max_event_bytes,
				self.max_held_bytes,
				&mut |event| conversion.event(event),
			);
			if let Err(next) = pushed {
				self.conversion.fail();
				error.get_or_insert(next);
			}
		}
		match error {
			Some(error) => Err(error),
			None => Ok(()),
		}
	}

	/// End of provider input. Missing terminal events and truncated SSE fail client
	/// output. A terminal event already seen ends processing, including within a chunk.
	pub fn finish(&mut self) -> Result<(), StreamError> {
		if self.is_finished() {
			return Ok(());
		}
		self.conversion.fail();
		self.sse.finish()?;
		self.decoder.finish()?;
		Ok(())
	}

	/// A transport failure. Later provider usage may still be fed if available.
	pub fn fail(&mut self) {
		self.conversion.fail();
	}

	#[must_use]
	pub fn is_finished(&self) -> bool {
		self.decoder.is_finished()
	}

	#[must_use]
	pub const fn usage(&self) -> Usage {
		self.conversion.usage.total()
	}

	#[must_use]
	pub const fn telemetry(&self) -> &Telemetry {
		&self.conversion.telemetry
	}

	/// An error before output replaces the entire HTTP reply. No stream bytes may
	/// be sent when this returns `Some`.
	#[must_use]
	pub fn early_failure(&self) -> Option<HttpReply> {
		self.conversion.writer.early_failure()
	}

	/// Whether output waits for the caller's check. Check after each `push` and after
	/// `finish`; chunks pushed while held are decoded and counted, and their output
	/// waits, up to [`OutputChecks::max_held_bytes`].
	pub fn progress(&mut self) -> ReplyProgress<'_> {
		ReplyProgress::of(&mut self.conversion)
	}

	/// Take bytes after each chunk. Early failures are exposed separately and never
	/// returned as stream output.
	pub fn take_output(&mut self) -> Option<Vec<u8>> {
		if self.early_failure().is_some() {
			return None;
		}
		let out = self.conversion.writer.take_output();
		(!out.is_empty()).then_some(out)
	}
}

/// Reads a provider's reply for its usage while the caller sends it on unchanged, for a
/// request whose provider serves the client's own protocol. Read a whole body with `read`,
/// or a stream with `push` and `finish`.
pub struct ObservedReply<'a> {
	upstream: UpstreamProtocol,
	model: &'a str,
	decoder: Decoder,
	sse: SseParser,
	usage: UsageFold,
	telemetry: Telemetry,
	max_event_bytes: usize,
	max_held_bytes: usize,
	pushed: bool,
}

impl<'a> ObservedReply<'a> {
	/// `model` is the one sent upstream; a reply that names none takes it. Limits are as for
	/// [`StreamReply`]: an event or part past them stops the reading, not the reply.
	#[must_use]
	pub fn new(
		upstream: UpstreamProtocol,
		model: &'a str,
		max_event_bytes: usize,
		max_held_bytes: usize,
	) -> Self {
		Self {
			upstream,
			model,
			decoder: Decoder::new(upstream, model, OBSERVED_REPLY_ID),
			sse: SseParser::new(max_event_bytes),
			usage: UsageFold::default(),
			telemetry: Telemetry::default(),
			max_event_bytes,
			max_held_bytes,
			pushed: false,
		}
	}

	/// Reads one whole reply body.
	pub fn read(&mut self, body: &[u8]) -> Result<(), DecodeError> {
		assert!(
			!self.pushed,
			"a reply is read whole or as a stream, not both"
		);
		let Self {
			usage, telemetry, ..
		} = self;
		let sink: &mut dyn FnMut(Event<'_>) = &mut |event| {
			usage.event(&event);
			telemetry.event(&event);
		};
		match self.upstream {
			UpstreamProtocol::Messages => messages::decoder::decode_buffered(body, sink),
			UpstreamProtocol::Responses { .. } => responses::decoder::decode_buffered(body, sink),
			UpstreamProtocol::ChatCompletions => {
				chat_completions::decoder::decode_buffered(body, self.model, OBSERVED_REPLY_ID, sink)
			},
			UpstreamProtocol::Gemini => {
				gemini::decoder::decode_buffered(body, self.model, OBSERVED_REPLY_ID, sink)
			},
		}
	}

	/// Feeds one transport chunk of a stream. Returns the first decode error in the chunk
	/// after reading its remaining events.
	pub fn push(&mut self, chunk: &[u8]) -> Result<(), DecodeError> {
		self.pushed = true;
		if self.decoder.is_finished() {
			return Ok(());
		}
		self.sse.push(chunk);
		let mut error = None;
		while !self.decoder.is_finished()
			&& let Some(frame) = self.sse.next_frame()
		{
			let Self {
				usage, telemetry, ..
			} = self;
			let pushed = self.decoder.push(
				frame,
				self.max_event_bytes,
				self.max_held_bytes,
				&mut |event| {
					usage.event(&event);
					telemetry.event(&event);
				},
			);
			if let Err(next) = pushed {
				error.get_or_insert(next);
			}
		}
		match error {
			Some(error) => Err(error),
			None => Ok(()),
		}
	}

	/// End of a stream. A stream that ended before its terminal event is reported.
	pub fn finish(&mut self) -> Result<(), StreamError> {
		if self.decoder.is_finished() {
			return Ok(());
		}
		self.sse.finish()?;
		self.decoder.finish()?;
		Ok(())
	}

	#[must_use]
	pub const fn usage(&self) -> Usage {
		self.usage.total()
	}

	#[must_use]
	pub const fn telemetry(&self) -> &Telemetry {
		&self.telemetry
	}
}

/// Nothing an observed reply decodes is written, so no reply or call needs its own id.
const OBSERVED_REPLY_ID: &str = "";

/// Whether reply conversion is ready for more input or waiting for the caller's check.
#[must_use]
pub enum ReplyProgress<'h> {
	Ready,
	Waiting(HeldReply<'h>),
}

/// Exclusive access to a reply while it waits for the caller's check.
pub struct HeldReply<'h> {
	reply: &'h mut (dyn ReplyHold + Send + 'h),
}

impl<'h> ReplyProgress<'h> {
	fn of(reply: &'h mut (dyn ReplyHold + Send + 'h)) -> Self {
		if reply.is_held() {
			ReplyProgress::Waiting(HeldReply { reply })
		} else {
			ReplyProgress::Ready
		}
	}
}

impl<'h> HeldReply<'h> {
	/// What the caller checks before the held output can go on.
	#[must_use]
	pub fn check(&self) -> Check<'_> {
		self.reply.check()
	}

	/// Releasing replays what arrived while held, which may hold again.
	pub fn resolve(self, verdict: Verdict) -> ReplyProgress<'h> {
		self.reply.resolve(verdict);
		ReplyProgress::of(self.reply)
	}
}

trait ReplyHold {
	fn is_held(&self) -> bool;
	fn check(&self) -> Check<'_>;
	fn resolve(&mut self, verdict: Verdict);
}

impl<W: ClientWriter> ReplyHold for Conversion<W> {
	fn is_held(&self) -> bool {
		Self::is_held(self)
	}

	fn check(&self) -> Check<'_> {
		match self.checks.as_ref().and_then(Checks::waiting) {
			Some(check) => check,
			None => unreachable!("only a held reply has a check"),
		}
	}

	fn resolve(&mut self, verdict: Verdict) {
		let Self { checks, writer, .. } = self;
		let Some(checks) = checks else {
			unreachable!("only a held reply can be resolved");
		};
		checks.resolve(verdict, &mut |event| writer.event(&event));
		if checks.failed() {
			writer.fail();
		}
	}
}

struct Conversion<W> {
	usage: UsageFold,
	telemetry: Telemetry,
	rewrites: ReplyRewrites,
	checks: Option<Checks>,
	stop: StopAt,
	writer: W,
}

trait ClientWriter {
	fn event(&mut self, event: &Event<'_>);
	fn fail(&mut self);
}

impl<W: ClientWriter> Conversion<W> {
	fn new(stop_sequences: Vec<String>, writer: W) -> Self {
		Self {
			usage: UsageFold::default(),
			telemetry: Telemetry::default(),
			rewrites: ReplyRewrites::default(),
			checks: None,
			stop: StopAt::new(stop_sequences),
			writer,
		}
	}

	fn event(&mut self, event: Event<'_>) {
		self.usage.event(&event);
		self.telemetry.event(&event);
		let Self {
			rewrites,
			checks,
			stop,
			writer,
			..
		} = self;
		// A provider that takes the stop sequence ends there, so nothing after it reaches a
		// rewrite or an output check.
		stop.event(event, &mut |event| {
			rewrites.event(event, &mut |event| match checks {
				Some(checks) => checks.event(event, &mut |event| writer.event(&event)),
				None => writer.event(&event),
			})
		});
		if self.checks.as_ref().is_some_and(Checks::failed) {
			self.writer.fail();
		}
	}

	fn is_held(&self) -> bool {
		self
			.checks
			.as_ref()
			.is_some_and(|checks| checks.waiting().is_some())
	}

	fn fail(&mut self) {
		self.writer.fail();
	}
}

fn read_failed() -> Failure {
	Failure::new(ErrorType::Api, READ_FAILED)
}

enum MessagesSequencer<'a> {
	Plain(Sequencer<'a>),
	Gemini(GeminiMessagesCompat<'a>),
}

impl<'a> MessagesSequencer<'a> {
	fn new(pair: &ReplyContext<'a>) -> Self {
		if pair.upstream == UpstreamProtocol::Gemini {
			Self::Gemini(GeminiMessagesCompat::new(pair.scope, pair.max_held_bytes))
		} else {
			Self::Plain(Sequencer::new(pair.scope, pair.max_held_bytes))
		}
	}

	fn event(&mut self, event: &Event<'_>, sink: &mut dyn Sink) {
		match self {
			Self::Plain(sequencer) => sequencer.event(event, sink),
			Self::Gemini(sequencer) => sequencer.event(event, sink),
		}
	}

	fn fail(&mut self, failure: &Failure, sink: &mut dyn Sink) {
		match self {
			Self::Plain(sequencer) => sequencer.fail(failure, sink),
			Self::Gemini(sequencer) => sequencer.fail(failure, sink),
		}
	}
}

enum StreamWriter<'a> {
	Messages {
		sequencer: MessagesSequencer<'a>,
		writer: SseWriter,
	},
	Responses(ResponseStream<'a>),
	Chat(ChatStream<'a>),
	Gemini(GeminiStream<'a>),
}

impl<'a> StreamWriter<'a> {
	fn new(pair: &ReplyContext<'a>) -> Self {
		let ReplyContext {
			scope,
			routes,
			created,
			max_held_bytes,
			..
		} = *pair;
		match pair.client {
			ClientProtocol::Messages => Self::Messages {
				sequencer: MessagesSequencer::new(pair),
				writer: SseWriter::default(),
			},
			ClientProtocol::Responses => Self::Responses(
				ResponseStream::new(scope, routes, created, max_held_bytes)
					.show_reasoning(pair.show_reasoning),
			),
			ClientProtocol::ChatCompletions => Self::Chat(
				ChatStream::new(scope, routes, created, max_held_bytes).include_usage(pair.include_usage),
			),
			ClientProtocol::Gemini => {
				Self::Gemini(GeminiStream::new(scope, max_held_bytes).show_thoughts(pair.show_reasoning))
			},
		}
	}

	fn take_output(&mut self) -> Vec<u8> {
		match self {
			Self::Messages { writer, .. } => std::mem::take(&mut writer.out),
			Self::Responses(writer) => {
				let mut out = Vec::new();
				while writer.progress() == WriterProgress::HasOutput {
					out.extend(writer.take_stream());
				}
				out
			},
			Self::Chat(writer) => writer.take_output(),
			Self::Gemini(writer) => {
				let mut out = Vec::new();
				while writer.progress() == WriterProgress::HasOutput {
					out.extend(writer.take_stream());
				}
				out
			},
		}
	}

	fn early_failure(&self) -> Option<HttpReply> {
		match self {
			Self::Messages { writer, .. } => writer.early_failure(),
			Self::Responses(writer) => writer.early_failure(),
			Self::Chat(writer) => writer.early_failure(),
			Self::Gemini(writer) => writer.early_failure(),
		}
	}
}

impl ClientWriter for StreamWriter<'_> {
	fn event(&mut self, event: &Event<'_>) {
		match self {
			Self::Messages { sequencer, writer } => sequencer.event(event, writer),
			Self::Responses(writer) => writer.event(event),
			Self::Chat(writer) => writer.event(event),
			Self::Gemini(writer) => writer.event(event),
		}
	}

	fn fail(&mut self) {
		match self {
			Self::Messages { sequencer, writer } => sequencer.fail(&read_failed(), writer),
			Self::Responses(writer) => writer.fail(ErrorType::Api.status()),
			Self::Chat(writer) => writer.fail(ErrorType::Api.status()),
			Self::Gemini(writer) => writer.fail(ErrorType::Api.status()),
		}
	}
}

enum BufferedWriter<'a> {
	Messages {
		sequencer: MessagesSequencer<'a>,
		writer: ReplyWriter,
	},
	Responses(BufferedResponse<'a>),
	Chat(BufferedChat<'a>),
	Gemini(GeminiBuffered<'a>),
}

impl<'a> BufferedWriter<'a> {
	fn new(pair: &ReplyContext<'a>) -> Self {
		let ReplyContext {
			scope,
			routes,
			created,
			max_held_bytes,
			..
		} = *pair;
		match pair.client {
			ClientProtocol::Messages => Self::Messages {
				sequencer: MessagesSequencer::new(pair),
				writer: ReplyWriter::new(max_held_bytes),
			},
			ClientProtocol::Responses => Self::Responses(
				BufferedResponse::new(scope, routes, created, max_held_bytes)
					.show_reasoning(pair.show_reasoning),
			),
			ClientProtocol::ChatCompletions => {
				Self::Chat(BufferedChat::new(scope, routes, created, max_held_bytes))
			},
			ClientProtocol::Gemini => {
				Self::Gemini(GeminiBuffered::new(scope, max_held_bytes).show_thoughts(pair.show_reasoning))
			},
		}
	}

	fn into_reply(self) -> HttpReply {
		match self {
			Self::Messages { writer, .. } => writer.finish(),
			Self::Responses(writer) => writer.into_reply(),
			Self::Chat(writer) => writer.into_reply(),
			Self::Gemini(writer) => writer.into_reply(),
		}
	}
}

impl ClientWriter for BufferedWriter<'_> {
	fn event(&mut self, event: &Event<'_>) {
		match self {
			Self::Messages { sequencer, writer } => sequencer.event(event, writer),
			Self::Responses(writer) => writer.event(event),
			Self::Chat(writer) => writer.event(event),
			Self::Gemini(writer) => writer.event(event),
		}
	}

	fn fail(&mut self) {
		match self {
			Self::Messages { sequencer, writer } => sequencer.fail(&read_failed(), writer),
			Self::Responses(writer) => writer.fail(ErrorType::Api.status()),
			Self::Chat(writer) => writer.fail(ErrorType::Api.status()),
			Self::Gemini(writer) => writer.fail(ErrorType::Api.status()),
		}
	}
}

enum Decoder {
	Messages(MessagesDecoder),
	Responses(ResponsesDecoder),
	Chat(CompletionsDecoder),
	Gemini(GeminiDecoder),
}

impl Decoder {
	fn new(upstream: UpstreamProtocol, model: &str, fallback_reply_id: &str) -> Self {
		match upstream {
			UpstreamProtocol::Messages => Self::Messages(MessagesDecoder::default()),
			UpstreamProtocol::Responses { .. } => Self::Responses(ResponsesDecoder::default()),
			UpstreamProtocol::ChatCompletions => {
				Self::Chat(CompletionsDecoder::new(model, fallback_reply_id))
			},
			UpstreamProtocol::Gemini => Self::Gemini(GeminiDecoder::new(model, fallback_reply_id)),
		}
	}

	/// Only a Responses provider sends an event too large to hold, its final one;
	/// from any other provider such an event fails the reply.
	fn push(
		&mut self,
		frame: Frame<'_>,
		max_event_bytes: usize,
		max_held_bytes: usize,
		sink: &mut dyn FnMut(Event<'_>),
	) -> Result<(), DecodeError> {
		let too_large = DecodeError::EventTooLarge {
			limit: max_event_bytes,
		};
		match frame {
			Frame::Event(event) => match self {
				Self::Responses(decoder) => decoder.push_json(event.data, sink),
				Self::Messages(decoder) => decoder.push_json(event.data, max_held_bytes, sink),
				Self::Chat(decoder) => decoder.push_json(event.data, max_held_bytes, sink),
				Self::Gemini(decoder) => decoder.push_json(event.data, max_held_bytes, sink),
			},
			Frame::Piece(piece) => match self {
				Self::Responses(decoder) => decoder.push_json_piece(piece, max_event_bytes),
				_ => Err(too_large),
			},
			Frame::PiecesEnd => match self {
				Self::Responses(decoder) => decoder.end_json_pieces(max_event_bytes, sink),
				_ => Ok(()),
			},
		}
	}

	fn is_finished(&self) -> bool {
		match self {
			Self::Messages(decoder) => decoder.is_finished(),
			Self::Responses(decoder) => decoder.is_finished(),
			Self::Chat(decoder) => decoder.is_finished(),
			Self::Gemini(decoder) => decoder.is_finished(),
		}
	}

	fn finish(&self) -> Result<(), DecodeError> {
		match self {
			Self::Messages(decoder) => decoder.finish(),
			Self::Responses(decoder) => decoder.finish(),
			Self::Chat(decoder) => decoder.finish(),
			Self::Gemini(decoder) => decoder.finish(),
		}
	}
}
