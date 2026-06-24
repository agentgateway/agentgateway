//! Streaming guardrail infrastructure: `GuardedSseBody` and per-guardrail evaluators.
//!
//! `GuardedSseBody` implements **windowed guardrail evaluation**:
//!
//! 1. Incoming SSE byte frames are held (not forwarded) while text deltas are
//!    accumulated into a pending batch.
//! 2. When the batch reaches `eval_threshold` bytes of text (or the stream ends),
//!    all `StreamingEvaluator`s are run against a window consisting of an
//!    *overlap tail* from previously evaluated text plus the pending batch. The
//!    overlap ensures patterns spanning a batch boundary are still seen
//!    contiguously by at least one evaluation.
//! 3. **Pass** → the held frames are flushed to the client and buffering resumes.
//!    **Block** → the held (never-forwarded) frames are discarded and a synthetic
//!    SSE error event is emitted. Content flushed by earlier passing windows
//!    cannot be retracted — an accepted accuracy/latency tradeoff.
//!
//! This is not 100% accurate: a guard that needs full-response context, or a
//! pattern spanning more than the overlap window, can be missed.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use ::http::HeaderMap;
use bytes::Bytes;
use http_body::Frame;
use pin_project_lite::pin_project;
use tokio_sse_codec::{Event, Frame as SseFrame, SseDecoder};
use tokio_util::codec::Decoder;
use tracing::warn;

use super::{
	FailureMode, ResponseGuard, ResponseGuardKind, StreamingEvaluator, StreamingGuardrailOutcome,
};
use crate::llm::policy::PromptGuard;
use crate::proxy::httpproxy::PolicyClient;

/// Text bytes accumulated before triggering a guardrail evaluation.
/// Larger values reduce guardrail API calls but increase time-to-first-byte
/// and the amount of content discarded on a mid-stream block.
pub const DEFAULT_EVAL_THRESHOLD: usize = 1024;

/// Tail bytes of previously evaluated text prepended to each new window so
/// patterns spanning a batch boundary are still seen contiguously.
pub const OVERLAP_BYTES: usize = 256;

/// Return the last `max_bytes` of `s`, respecting UTF-8 char boundaries.
pub fn tail_chars(s: &str, max_bytes: usize) -> &str {
	if s.len() <= max_bytes {
		return s;
	}
	let mut start = s.len() - max_bytes;
	while !s.is_char_boundary(start) {
		start += 1;
	}
	&s[start..]
}

/// Run all evaluators against a window. Returns the rejection body if any evaluator blocked.
pub async fn evaluate_window(
	evaluators: &mut [Box<dyn StreamingEvaluator>],
	window: &str,
) -> Option<Bytes> {
	for ev in evaluators.iter_mut() {
		match ev.evaluate(window).await {
			Ok(Some(StreamingGuardrailOutcome::Blocked(body))) => {
				tracing::debug!("streaming guardrail blocked response window");
				return Some(body);
			},
			Ok(None) => {},
			Err(e) => match ev.failure_mode() {
				FailureMode::FailClosed => {
					warn!("streaming guardrail error, failing closed: {e}");
					return Some(Bytes::from_static(b"Content blocked by guardrail policy"));
				},
				FailureMode::FailOpen => {
					warn!("streaming guardrail error, failing open: {e}");
				},
			},
		}
	}
	None
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Construct a boxed `StreamingEvaluator` for the given `ResponseGuard`.
pub fn make_evaluator(
	guard: &ResponseGuard,
	client: PolicyClient,
	http_headers: HeaderMap,
) -> Box<dyn StreamingEvaluator> {
	Box::new(ResponseGuardEvaluator {
		guard: guard.clone(),
		client,
		http_headers,
	})
}

struct ResponseGuardEvaluator {
	guard: ResponseGuard,
	client: PolicyClient,
	http_headers: HeaderMap,
}

#[async_trait::async_trait]
impl StreamingEvaluator for ResponseGuardEvaluator {
	fn failure_mode(&self) -> FailureMode {
		match &self.guard.kind {
			ResponseGuardKind::Webhook(wh) => wh.failure_mode,
			_ => FailureMode::FailOpen,
		}
	}

	async fn evaluate(&mut self, window: &str) -> anyhow::Result<Option<StreamingGuardrailOutcome>> {
		PromptGuard::evaluate_streaming_response_window(
			&self.guard,
			window,
			&self.client,
			&self.http_headers,
		)
		.await
	}
}

// ---------------------------------------------------------------------------
// GuardedSseBody
// ---------------------------------------------------------------------------

/// Synthetic SSE error event sent to the client when a guardrail blocks content.
///
/// Encoded as a single `data: {...}\n\n` SSE frame.
fn guardrail_blocked_sse_bytes(body: Bytes) -> Bytes {
	let message = String::from_utf8_lossy(&body);
	let event = serde_json::json!({
		"error": {
			"type": "guardrail_blocked",
			"code": "guardrail_intervention",
			"message": message,
		}
	});
	Bytes::from(format!("data: {event}\n\n"))
}

type EvalFuture =
	Pin<Box<dyn Future<Output = (Vec<Box<dyn StreamingEvaluator>>, Option<Bytes>)> + Send + 'static>>;

/// Internal state machine for `GuardedSseBody`.
enum GuardedBodyState {
	/// Reading from upstream, holding frames until the eval threshold is reached.
	Buffering,
	/// Evaluating the current window asynchronously. `eof` records whether the
	/// upstream is already exhausted (this is the final evaluation).
	Evaluating { fut: EvalFuture, eof: bool },
	/// Yield held frames in order, then return to `Buffering` (or `Done` if `eof`).
	Flushing { queue: VecDeque<Bytes>, eof: bool },
	/// Send the synthetic error event then close.
	Blocked(Bytes),
	/// Done – no more frames.
	Done,
}

pin_project! {
	// An `http_body::Body` wrapper that implements windowed guardrail evaluation.
	pub struct GuardedSseBody {
		#[pin]
		inner: crate::http::Body,
		evaluators: Vec<Box<dyn StreamingEvaluator>>,
		eval_threshold: usize,
		buffer_limit: usize,
		held_frames: Vec<Bytes>,
		held_bytes: usize,
		pending_text: String,
		overlap_tail: String,
		sse_decoder: SseDecoder<Bytes>,
		decode_buffer: bytes::BytesMut,
		state: GuardedBodyState,
		// Owns the rate-limit logger; dropped only when this body is fully consumed,
		// so telemetry is recorded at the correct time.
		logger: Option<crate::llm::AmendOnDrop>,
	}
}

impl GuardedSseBody {
	/// Create a new `GuardedSseBody` with the default evaluation threshold.
	///
	/// * `inner` – the upstream SSE body.
	/// * `evaluators` – one evaluator per configured response guard.
	/// * `buffer_limit` – max bytes of held frames; reaching it forces an evaluation.
	/// * `logger` – rate-limit logger that must outlive the streaming response.
	// We do actually return Self; just wrapped in an http_body::Body. The annotation silences a false positive from clippy about that.
	#[allow(clippy::new_ret_no_self)]
	pub fn new(
		inner: crate::http::Body,
		evaluators: Vec<Box<dyn StreamingEvaluator>>,
		buffer_limit: usize,
		logger: Option<crate::llm::AmendOnDrop>,
	) -> crate::http::Body {
		Self::with_threshold(
			inner,
			evaluators,
			buffer_limit,
			logger,
			DEFAULT_EVAL_THRESHOLD,
		)
	}

	/// Like [`GuardedSseBody::new`] but with an explicit evaluation threshold.
	pub fn with_threshold(
		inner: crate::http::Body,
		evaluators: Vec<Box<dyn StreamingEvaluator>>,
		buffer_limit: usize,
		logger: Option<crate::llm::AmendOnDrop>,
		eval_threshold: usize,
	) -> crate::http::Body {
		crate::http::Body::new(Self {
			inner,
			evaluators,
			eval_threshold,
			buffer_limit,
			held_frames: Vec::new(),
			held_bytes: 0,
			pending_text: String::new(),
			overlap_tail: String::new(),
			sse_decoder: SseDecoder::with_max_size(buffer_limit),
			decode_buffer: bytes::BytesMut::new(),
			state: GuardedBodyState::Buffering,
			logger,
		})
	}

	/// Extract text delta from a parsed SSE frame if present.
	fn extract_text_delta(frame: SseFrame<Bytes>) -> Option<String> {
		let SseFrame::Event(Event { data, .. }) = frame else {
			return None;
		};
		if data.as_ref() == b"[DONE]" {
			return None;
		}
		if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&data) {
			// OpenAI responses: response.output_text.delta
			if v.get("type").and_then(|t| t.as_str()) == Some("response.output_text.delta")
				&& let Some(text) = v.get("delta").and_then(|s| s.as_str())
			{
				return Some(text.to_string());
			}
			// OpenAI completions: choices[0].delta.content
			if let Some(text) = v
				.get("choices")
				.and_then(|c| c.get(0))
				.and_then(|c| c.get("delta"))
				.and_then(|d| d.get("content"))
				.and_then(|s| s.as_str())
			{
				return Some(text.to_string());
			}
			// Anthropic messages: delta.text
			if let Some(text) = v
				.get("delta")
				.and_then(|d| d.get("text"))
				.and_then(|s| s.as_str())
			{
				return Some(text.to_string());
			}
		}
		None
	}
}

impl http_body::Body for GuardedSseBody {
	type Data = Bytes;
	type Error = crate::http::Error;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let mut this = self.project();

		loop {
			match this.state {
				// -----------------------------------------------------------------
				// Flushing: yield held frames one at a time.
				// -----------------------------------------------------------------
				GuardedBodyState::Flushing { queue, eof } => {
					if let Some(frame) = queue.pop_front() {
						return Poll::Ready(Some(Ok(Frame::data(frame))));
					} else if *eof {
						*this.state = GuardedBodyState::Done;
						return Poll::Ready(None);
					} else {
						*this.state = GuardedBodyState::Buffering;
					}
				},
				// -----------------------------------------------------------------
				// Blocked: yield synthetic error event, then done.
				// -----------------------------------------------------------------
				GuardedBodyState::Blocked(body) => {
					let body = std::mem::take(body);
					*this.state = GuardedBodyState::Done;
					return Poll::Ready(Some(Ok(Frame::data(guardrail_blocked_sse_bytes(body)))));
				},
				// -----------------------------------------------------------------
				// Done: stream exhausted.
				// -----------------------------------------------------------------
				GuardedBodyState::Done => {
					return Poll::Ready(None);
				},
				// -----------------------------------------------------------------
				// Evaluating: poll the guardrail future for the current window.
				// -----------------------------------------------------------------
				GuardedBodyState::Evaluating { fut, eof } => match fut.as_mut().poll(cx) {
					Poll::Pending => return Poll::Pending,
					Poll::Ready((evaluators, blocked_body)) => {
						*this.evaluators = evaluators;
						if let Some(body) = blocked_body {
							this.held_frames.clear();
							*this.held_bytes = 0;
							*this.state = GuardedBodyState::Blocked(body);
						} else {
							let queue: VecDeque<Bytes> = this.held_frames.drain(..).collect();
							*this.held_bytes = 0;
							*this.state = GuardedBodyState::Flushing { queue, eof: *eof };
						}
					},
				},
				// -----------------------------------------------------------------
				// Buffering: read from upstream, holding frames until threshold.
				// -----------------------------------------------------------------
				GuardedBodyState::Buffering => {
					match this.inner.as_mut().poll_frame(cx) {
						Poll::Pending => return Poll::Pending,
						Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
						Poll::Ready(Some(Ok(frame))) => {
							let Some(data) = frame.data_ref() else {
								return Poll::Ready(Some(Ok(frame)));
							};

							let raw = data.clone();
							*this.held_bytes += raw.len();
							this.held_frames.push(raw.clone());

							this.decode_buffer.extend_from_slice(&raw);
							loop {
								match this.sse_decoder.decode(this.decode_buffer) {
									Ok(Some(sse_frame)) => {
										if let Some(delta) = GuardedSseBody::extract_text_delta(sse_frame) {
											this.pending_text.push_str(&delta);
										}
									},
									Ok(None) => break,
									Err(e) => {
										// Clear the buffer and reset the decoder so invalid bytes
										// don't accumulate across chunks and cause repeated errors.
										warn!("SSE decode error in streaming guardrail body, resetting decoder: {e}");
										this.decode_buffer.clear();
										*this.sse_decoder = SseDecoder::with_max_size(*this.buffer_limit);
										break;
									},
								}
							}

							let over_limit = *this.held_bytes >= *this.buffer_limit;
							if this.pending_text.len() >= *this.eval_threshold || over_limit {
								// Having a full buffer but empty text implies that the
								// buffer is full of non-text frames (e.g. control frames or unsupported SSE formats that fail to decode).
								// In that case, flush the buffer as-is without evaluation, to avoid stalling on unprocessable content.
								if this.pending_text.is_empty() {
									let queue: VecDeque<Bytes> = this.held_frames.drain(..).collect();
									*this.held_bytes = 0;
									*this.state = GuardedBodyState::Flushing { queue, eof: false };
									continue;
								}
								let batch = std::mem::take(this.pending_text);
								let window = format!("{}{}", this.overlap_tail, batch);
								*this.overlap_tail = tail_chars(&window, OVERLAP_BYTES).to_string();
								let mut evaluators = std::mem::take(this.evaluators);
								let fut: EvalFuture = Box::pin(async move {
									let blocked_body = evaluate_window(&mut evaluators, &window).await;
									(evaluators, blocked_body)
								});
								*this.state = GuardedBodyState::Evaluating { fut, eof: false };
							}
						},
						Poll::Ready(None) => {
							loop {
								match this.sse_decoder.decode_eof(this.decode_buffer) {
									Ok(Some(sse_frame)) => {
										if let Some(delta) = GuardedSseBody::extract_text_delta(sse_frame) {
											this.pending_text.push_str(&delta);
										}
									},
									Ok(None) => break,
									Err(e) => {
										warn!("SSE decode error at EOF in streaming guardrail body: {e}");
										this.decode_buffer.clear();
										break;
									},
								}
							}

							if this.pending_text.is_empty() {
								let queue: VecDeque<Bytes> = this.held_frames.drain(..).collect();
								*this.held_bytes = 0;
								*this.state = GuardedBodyState::Flushing { queue, eof: true };
								continue;
							}

							let batch = std::mem::take(this.pending_text);
							let window = format!("{}{}", this.overlap_tail, batch);
							this.overlap_tail.clear();
							let mut evaluators = std::mem::take(this.evaluators);
							let fut: EvalFuture = Box::pin(async move {
								let blocked_body = evaluate_window(&mut evaluators, &window).await;
								(evaluators, blocked_body)
							});
							*this.state = GuardedBodyState::Evaluating { fut, eof: true };
						},
					}
				},
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use http_body_util::BodyExt as _;

	use super::*;

	struct PassEvaluator;

	#[async_trait::async_trait]
	impl StreamingEvaluator for PassEvaluator {
		async fn evaluate(
			&mut self,
			_window: &str,
		) -> anyhow::Result<Option<StreamingGuardrailOutcome>> {
			Ok(None)
		}
	}

	struct BlockEvaluator;

	#[async_trait::async_trait]
	impl StreamingEvaluator for BlockEvaluator {
		async fn evaluate(
			&mut self,
			_window: &str,
		) -> anyhow::Result<Option<StreamingGuardrailOutcome>> {
			Ok(Some(StreamingGuardrailOutcome::Blocked(
				Bytes::from_static(b"blocked"),
			)))
		}
	}

	struct PatternEvaluator {
		pattern: regex::Regex,
	}

	#[async_trait::async_trait]
	impl StreamingEvaluator for PatternEvaluator {
		async fn evaluate(
			&mut self,
			window: &str,
		) -> anyhow::Result<Option<StreamingGuardrailOutcome>> {
			if self.pattern.is_match(window) {
				return Ok(Some(StreamingGuardrailOutcome::Blocked(
					Bytes::from_static(b"blocked"),
				)));
			}
			Ok(None)
		}
	}

	struct ErrorEvaluator {
		mode: crate::llm::policy::FailureMode,
	}

	#[async_trait::async_trait]
	impl StreamingEvaluator for ErrorEvaluator {
		fn failure_mode(&self) -> crate::llm::policy::FailureMode {
			self.mode
		}

		async fn evaluate(
			&mut self,
			_window: &str,
		) -> anyhow::Result<Option<StreamingGuardrailOutcome>> {
			Err(anyhow::anyhow!("simulated evaluator error"))
		}
	}

	fn sse_bytes(content: &str) -> Bytes {
		Bytes::from(format!("data: {}\n\n", content))
	}

	fn delta_bytes(text: &str) -> Bytes {
		sse_bytes(&format!(
			"{{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}",
			text
		))
	}

	fn make_body(chunks: Vec<Bytes>) -> crate::http::Body {
		use std::convert::Infallible;

		use futures_util::stream;
		let stream = stream::iter(chunks.into_iter().map(Ok::<Bytes, Infallible>));
		crate::http::Body::from_stream(stream)
	}

	fn contains(haystack: &[u8], needle: &[u8]) -> bool {
		haystack.windows(needle.len()).any(|w| w == needle)
	}

	#[tokio::test]
	async fn test_pass_through() {
		let chunk = delta_bytes("hello");
		let done = sse_bytes("[DONE]");
		let body = make_body(vec![chunk.clone(), done.clone()]);

		let guarded = GuardedSseBody::new(body, vec![Box::new(PassEvaluator)], 1024 * 1024, None);

		let bytes = guarded.collect().await.unwrap().to_bytes();
		assert!(bytes.starts_with(&chunk));
	}

	#[tokio::test]
	async fn test_block() {
		let chunk = delta_bytes("bad content");
		let done = sse_bytes("[DONE]");
		let body = make_body(vec![chunk, done]);

		let guarded = GuardedSseBody::new(body, vec![Box::new(BlockEvaluator)], 1024 * 1024, None);

		let bytes = guarded.collect().await.unwrap().to_bytes();
		assert!(contains(&bytes, b"guardrail_blocked"));
		assert!(!contains(&bytes, b"bad content"));
	}

	fn pattern_evaluator(pattern: &str) -> PatternEvaluator {
		PatternEvaluator {
			pattern: regex::Regex::new(pattern).unwrap(),
		}
	}

	#[tokio::test]
	async fn test_regex_blocks_matching_response() {
		let chunk = delta_bytes("my SSN is 123-45-6789");
		let done = sse_bytes("[DONE]");
		let body = make_body(vec![chunk, done]);

		let guarded = GuardedSseBody::new(
			body,
			vec![Box::new(pattern_evaluator("SSN"))],
			1024 * 1024,
			None,
		);

		let bytes = guarded.collect().await.unwrap().to_bytes();
		assert!(contains(&bytes, b"guardrail_blocked"));
		assert!(!contains(&bytes, b"SSN"));
	}

	#[tokio::test]
	async fn test_regex_passes_non_matching_response() {
		let chunk = delta_bytes("hello world");
		let done = sse_bytes("[DONE]");
		let body = make_body(vec![chunk.clone(), done]);

		let guarded = GuardedSseBody::new(
			body,
			vec![Box::new(pattern_evaluator("SSN"))],
			1024 * 1024,
			None,
		);

		let bytes = guarded.collect().await.unwrap().to_bytes();
		assert!(bytes.starts_with(&chunk));
		assert!(!contains(&bytes, b"guardrail_blocked"));
	}

	#[tokio::test]
	async fn test_regex_accumulates_within_batch() {
		let chunk1 = delta_bytes("my credit");
		let chunk2 = delta_bytes(" card");
		let done = sse_bytes("[DONE]");
		let body = make_body(vec![chunk1, chunk2, done]);

		let guarded = GuardedSseBody::new(
			body,
			vec![Box::new(pattern_evaluator("credit card"))],
			1024 * 1024,
			None,
		);

		let bytes = guarded.collect().await.unwrap().to_bytes();
		assert!(contains(&bytes, b"guardrail_blocked"));
		assert!(!contains(&bytes, b"credit"));
	}

	#[tokio::test]
	async fn test_windowed_incremental_flush_then_block() {
		let chunk1 = delta_bytes("this part is fine");
		let chunk2 = delta_bytes("forbidden words here");
		let done = sse_bytes("[DONE]");
		let body = make_body(vec![chunk1.clone(), chunk2, done]);

		let guarded = GuardedSseBody::with_threshold(
			body,
			vec![Box::new(pattern_evaluator("forbidden"))],
			1024 * 1024,
			None,
			4,
		);

		let bytes = guarded.collect().await.unwrap().to_bytes();
		assert!(contains(&bytes, b"this part is fine"));
		assert!(!contains(&bytes, b"forbidden"));
		assert!(contains(&bytes, b"guardrail_blocked"));
	}

	#[tokio::test]
	async fn test_overlap_catches_boundary_spanning_pattern() {
		let chunk1 = delta_bytes("my credit");
		let chunk2 = delta_bytes(" card number");
		let done = sse_bytes("[DONE]");
		let body = make_body(vec![chunk1, chunk2, done]);

		let guarded = GuardedSseBody::with_threshold(
			body,
			vec![Box::new(pattern_evaluator("credit card"))],
			1024 * 1024,
			None,
			4,
		);

		let bytes = guarded.collect().await.unwrap().to_bytes();
		assert!(contains(&bytes, b"guardrail_blocked"));
		assert!(!contains(&bytes, b"card number"));
	}

	#[test]
	fn test_tail_chars_respects_utf8_boundaries() {
		let s = "héllo wörld";
		let t = tail_chars(s, 4);
		assert!(t.len() <= 4);
		assert!(s.ends_with(t));
		let s2 = "aé";
		assert_eq!(tail_chars(s2, 1), "");
		assert_eq!(tail_chars(s2, 2), "é");
	}

	#[tokio::test]
	async fn evaluate_window_fail_closed_blocks_on_error() {
		use crate::llm::policy::FailureMode;
		let mut evs: Vec<Box<dyn StreamingEvaluator>> = vec![Box::new(ErrorEvaluator {
			mode: FailureMode::FailClosed,
		})];
		assert_eq!(
			evaluate_window(&mut evs, "some text").await.as_deref(),
			Some(&b"Content blocked by guardrail policy"[..])
		);
	}

	#[tokio::test]
	async fn evaluate_window_fail_open_passes_on_error() {
		use crate::llm::policy::FailureMode;
		let mut evs: Vec<Box<dyn StreamingEvaluator>> = vec![Box::new(ErrorEvaluator {
			mode: FailureMode::FailOpen,
		})];
		assert!(evaluate_window(&mut evs, "some text").await.is_none());
	}
}
