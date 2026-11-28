use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{Error, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use async_openai::types::realtime::{
	RealtimeClientEvent, RealtimeResponseUsage, RealtimeServerEvent, UserMessageContent,
};
use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use websocket_sans_io::{FrameInfo, Opcode, WebsocketFrameEncoder, WebsocketFrameEvent};

use crate::llm::policy::PromptGuard;
use crate::llm::{LLMInfo, UsageLine};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::AsyncLog;

fn add_usage_value(target: &mut Option<u64>, value: Option<u64>) {
	if let Some(value) = value {
		*target = Some(target.unwrap_or_default().saturating_add(value));
	}
}

/// Accumulate usage reported for a Realtime response into the connection-level record.
/// Realtime sockets can carry multiple responses, so each response contributes to the
/// connection total instead of replacing usage from earlier turns.
fn accumulate_realtime_usage(log: &AsyncLog<LLMInfo>, usage: &serde_json::Value) {
	log.non_atomic_mutate(|info| {
		let response = &mut info.response;
		add_usage_value(
			&mut response.input_tokens,
			usage.get("input_tokens").and_then(serde_json::Value::as_u64),
		);
		add_usage_value(
			&mut response.output_tokens,
			usage.get("output_tokens").and_then(serde_json::Value::as_u64),
		);
		add_usage_value(
			&mut response.total_tokens,
			usage.get("total_tokens").and_then(serde_json::Value::as_u64),
		);

		let input = usage
			.get("input_token_details")
			.or_else(|| usage.get("input_tokens_details"));
		add_usage_value(
			&mut response.input_text_tokens,
			input
				.and_then(|v| v.get("text_tokens"))
				.and_then(serde_json::Value::as_u64),
		);
		add_usage_value(
			&mut response.input_image_tokens,
			input
				.and_then(|v| v.get("image_tokens"))
				.and_then(serde_json::Value::as_u64),
		);
		add_usage_value(
			&mut response.input_audio_tokens,
			input
				.and_then(|v| v.get("audio_tokens"))
				.and_then(serde_json::Value::as_u64),
		);
		add_usage_value(
			&mut response.cached_input_tokens,
			input
				.and_then(|v| v.get("cached_tokens"))
				.and_then(serde_json::Value::as_u64),
		);

		let output = usage
			.get("output_token_details")
			.or_else(|| usage.get("output_tokens_details"));
		add_usage_value(
			&mut response.output_text_tokens,
			output
				.and_then(|v| v.get("text_tokens"))
				.and_then(serde_json::Value::as_u64),
		);
		add_usage_value(
			&mut response.output_audio_tokens,
			output
				.and_then(|v| v.get("audio_tokens"))
				.and_then(serde_json::Value::as_u64),
		);
	});
}

/// Record GPT-Live lifecycle usage and delegated Responses usage carried in Live events.
/// Live duration updates are cumulative snapshots, while each completed delegated Response
/// contributes a separate token-usage line.
fn record_live_usage(
	log: &AsyncLog<LLMInfo>,
	event: &serde_json::Value,
	seen_responses: &mut HashSet<String>,
) {
	match event.get("type").and_then(serde_json::Value::as_str) {
		Some("session.started") => {
			if let Some(model) = event
				.get("session")
				.and_then(|session| session.get("model"))
				.and_then(serde_json::Value::as_str)
			{
				log.non_atomic_mutate(|info| {
					info.response.provider_model = Some(agent_core::strng::new(model));
				});
			}
		},
		Some("session.usage.updated" | "session.closed") => {
			log.non_atomic_mutate(|info| {
				if let Some(seconds) = event
					.get("usage")
					.and_then(|usage| usage.get("seconds"))
					.and_then(serde_json::Value::as_f64)
				{
					// Live usage is cumulative; each update replaces the previous snapshot.
					info.response.audio_seconds = Some(seconds);
				}
				if event.get("type").and_then(serde_json::Value::as_str) == Some("session.closed")
					&& let Some(reason) = event.get("reason").and_then(serde_json::Value::as_str)
				{
					info.response.finish_reasons = Some(vec![agent_core::strng::new(reason)]);
				}
			});
		},
		Some("response.event") => {
			let nested = event.get("event");
			if nested
				.and_then(|value| value.get("type"))
				.and_then(serde_json::Value::as_str)
				!= Some("response.completed")
			{
				return;
			}
			let Some(response) = nested.and_then(|value| value.get("response")) else {
				return;
			};
			let Some(response_id) = response.get("id").and_then(serde_json::Value::as_str) else {
				return;
			};
			if !seen_responses.insert(response_id.to_string()) {
				return;
			}
			let Some(usage) = response.get("usage") else {
				return;
			};
			let model = response
				.get("model")
				.and_then(serde_json::Value::as_str)
				.map(agent_core::strng::new);
			let input_tokens = usage.get("input_tokens").and_then(serde_json::Value::as_u64);
			let output_tokens = usage.get("output_tokens").and_then(serde_json::Value::as_u64);
			let cached_input_tokens = usage
				.get("input_tokens_details")
				.and_then(|details| details.get("cached_tokens"))
				.and_then(serde_json::Value::as_u64);
			let input_audio_tokens = usage
				.get("input_tokens_details")
				.and_then(|details| details.get("audio_tokens"))
				.and_then(serde_json::Value::as_u64);
			let output_audio_tokens = usage
				.get("output_tokens_details")
				.and_then(|details| details.get("audio_tokens"))
				.and_then(serde_json::Value::as_u64);

			if let Ok(usage) = serde_json::to_value(usage) {
				accumulate_realtime_usage(log, &usage);
			}
			log.non_atomic_mutate(|info| {
				info.response.usage_lines.push(UsageLine {
					model,
					input_tokens: input_tokens.unwrap_or_default(),
					output_tokens: output_tokens.unwrap_or_default(),
					input_audio_tokens,
					output_audio_tokens,
					cached_input_tokens,
					..Default::default()
				});
			});
		},
		_ => {},
	}
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResponseDoneEvent {
	/// The response resource.
	pub response: ResponseResource,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResponseResource {
	/// Usage statistics for the response.
	pub usage: Option<RealtimeResponseUsage>,
}

struct Parser<IO> {
	inner: IO,
	frames: Option<WsFrameAccumulator>,
	log: AsyncLog<LLMInfo>,
	seen_live_responses: RefCell<HashSet<String>>,
}

impl<IO> Parser<IO> {
	fn emit(&self, data: Bytes) {
		let Ok(data) = str::from_utf8(&data) else {
			return;
		};
		let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
			return;
		};
		record_live_usage(&self.log, &value, &mut self.seen_live_responses.borrow_mut());
		if value.get("type").and_then(serde_json::Value::as_str) == Some("response.done") {
			let Ok(typed) = serde_json::from_value::<ResponseDoneEvent>(value) else {
				return;
			};
			if let Some(usage) = typed.response.usage {
				// TODO: do we need to parse the request side to get the request model?
				// it seems like we get an event from the server with the same thing.
				// also, the model can change... so what do we report??
				if let Ok(usage) = serde_json::to_value(usage) {
					accumulate_realtime_usage(&self.log, &usage);
				}
			}
		}
	}
}

impl<IO: AsyncWrite + Unpin + 'static> AsyncWrite for Parser<IO> {
	fn poll_write(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &[u8],
	) -> Poll<Result<usize, Error>> {
		Pin::new(&mut self.inner).poll_write(cx, buf)
	}

	fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut self.inner).poll_flush(cx)
	}

	fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
		Pin::new(&mut self.inner).poll_shutdown(cx)
	}

	fn poll_write_vectored(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		bufs: &[IoSlice<'_>],
	) -> Poll<Result<usize, Error>> {
		Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
	}

	fn is_write_vectored(&self) -> bool {
		self.inner.is_write_vectored()
	}
}
impl<IO: AsyncRead + Unpin + 'static> AsyncRead for Parser<IO> {
	fn poll_read(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		let orig = buf.filled().len();
		ready!(Pin::new(&mut self.inner).poll_read(cx, buf)?);
		if buf.filled().len() - orig == 0 {
			// EOF
			return Poll::Ready(Ok(()));
		}
		let Some(frames) = &mut self.frames else {
			return Poll::Ready(Ok(()));
		};
		frames.push(&buf.filled()[orig..]);
		let Ok(completed) = frames.drain_frames() else {
			self.frames = None;
			return Poll::Ready(Ok(()));
		};
		for frame in completed {
			if let WsCompletedFrame::Text { payload, .. } = frame {
				self.emit(payload);
			}
		}
		Poll::Ready(Ok(()))
	}
}

pub async fn parser<IO>(
	body: IO,
	log: AsyncLog<LLMInfo>,
	frame_limit: usize,
) -> impl AsyncRead + AsyncWrite + Unpin + 'static
where
	IO: AsyncRead + AsyncWrite + Unpin + 'static,
{
	Parser {
		inner: body,
		frames: Some(WsFrameAccumulator::new(frame_limit)),
		log,
		seen_live_responses: RefCell::new(HashSet::new()),
	}
}

// ---------------------------------------------------------------------------
// Guarded WebSocket realtime proxy
// ---------------------------------------------------------------------------

/// Encode a WebSocket text frame (server-side, unmasked) containing `payload`.
fn encode_ws_text_frame(payload: &[u8]) -> Bytes {
	let mut encoder = WebsocketFrameEncoder::new();
	let frame_info = FrameInfo {
		opcode: Opcode::Text,
		payload_length: payload.len() as u64,
		mask: None,
		fin: true,
		reserved: 0,
	};
	let header = encoder.start_frame(&frame_info);
	let mut out = BytesMut::with_capacity(header.len() + payload.len());
	out.extend_from_slice(&header);
	out.extend_from_slice(payload);
	out.freeze()
}

/// Encode a WebSocket text frame with `mask` (client-side, masked).
fn encode_ws_text_frame_masked(payload: &[u8], mask: [u8; 4]) -> Bytes {
	let mut encoder = WebsocketFrameEncoder::new();
	let frame_info = FrameInfo {
		opcode: Opcode::Text,
		payload_length: payload.len() as u64,
		mask: Some(mask),
		fin: true,
		reserved: 0,
	};
	let header = encoder.start_frame(&frame_info);
	let mut out = BytesMut::with_capacity(header.len() + payload.len());
	out.extend_from_slice(&header);
	let mut payload_copy = payload.to_vec();
	encoder.transform_frame_payload(&mut payload_copy);
	out.extend_from_slice(&payload_copy);
	out.freeze()
}

/// Synthetic WebSocket error event sent when a guardrail blocks content.
fn guardrail_blocked_ws_event_bytes(body: Bytes) -> Bytes {
	let message = String::from_utf8_lossy(&body);
	let event = serde_json::json!({
		"type": "error",
		"error": {
			"type": "guardrail_blocked",
			"code": "guardrail_intervention",
			"message": message,
		}
	});
	encode_ws_text_frame(event.to_string().as_bytes())
}

/// Synthetic `response.cancel` event sent to the server when a response guard blocks.
///
/// The WebSocket spec requires client→server frames to be masked. We use a zero mask
/// so the XOR is identity (payload bytes are unchanged) while the frame is formally masked.
fn response_cancel_event_bytes(mask: [u8; 4]) -> Bytes {
	// NOTE: response.cancel is sent without a response_id; the server cancels its most-recent
	// active response. Correctly targeting a specific response in the Realtime API's out-of-band
	// (concurrent) response pattern requires a response_id here and per-response blocked tracking
	// — see https://github.com/agentgateway/agentgateway/issues/2213.
	let json = r#"{"type":"response.cancel"}"#;
	encode_ws_text_frame_masked(json.as_bytes(), mask)
}

// ---------------------------------------------------------------------------
// WebSocket frame accumulator
// ---------------------------------------------------------------------------

struct WsFrameAccumulator {
	decoder: websocket_sans_io::WebsocketFrameDecoder,
	frame_limit: usize,
	pending: BytesMut,
	frame_raw: BytesMut,
	frame_payload: BytesMut,
}

enum WsCompletedFrame {
	Text { raw: Bytes, payload: Bytes },
	Other { raw: Bytes },
}

struct FrameTooLarge;

impl WsFrameAccumulator {
	fn new(frame_limit: usize) -> Self {
		Self {
			decoder: websocket_sans_io::WebsocketFrameDecoder::new(),
			frame_limit,
			pending: BytesMut::new(),
			frame_raw: BytesMut::new(),
			frame_payload: BytesMut::new(),
		}
	}

	fn push(&mut self, data: &[u8]) {
		self.pending.extend_from_slice(data);
	}

	fn drain_frames(&mut self) -> Result<Vec<WsCompletedFrame>, FrameTooLarge> {
		let mut result = Vec::new();
		// The decoder unmasks payloads in place; preserve pending for raw forwarding.
		let mut copy = self.pending.to_vec();
		let mut remaining = copy.as_mut_slice();
		loop {
			let ret = match self.decoder.add_data(remaining) {
				Ok(r) => r,
				Err(_) => {
					// Protocol error: forward all pending raw bytes as an opaque frame and
					// clear accumulator state so the proxy makes progress rather than
					// retrying the same bytes and stalling indefinitely.
					if !self.pending.is_empty() {
						let raw = self.pending.split().freeze();
						self.frame_raw.clear();
						self.frame_payload.clear();
						result.push(WsCompletedFrame::Other { raw });
					}
					break;
				},
			};
			if ret.consumed_bytes == 0 && ret.event.is_none() {
				break;
			}
			if let Some(WebsocketFrameEvent::Start { frame_info, .. }) = ret.event
				&& frame_info.payload_length > self.frame_limit as u64
			{
				return Err(FrameTooLarge);
			}

			let raw_chunk = self.pending.split_to(ret.consumed_bytes).freeze();
			self.frame_raw.extend_from_slice(&raw_chunk);

			match ret.event {
				Some(WebsocketFrameEvent::PayloadChunk {
					original_opcode: Opcode::Text,
				}) => {
					self
						.frame_payload
						.extend_from_slice(&remaining[..ret.consumed_bytes]);
				},
				Some(WebsocketFrameEvent::End {
					original_opcode: Opcode::Text,
					frame_info: FrameInfo { fin: true, .. },
				}) => {
					self
						.frame_payload
						.extend_from_slice(&remaining[..ret.consumed_bytes]);
					let raw = self.frame_raw.split().freeze();
					let payload = self.frame_payload.split().freeze();
					result.push(WsCompletedFrame::Text { raw, payload });
				},
				Some(WebsocketFrameEvent::End { .. }) => {
					let raw = self.frame_raw.split().freeze();
					self.frame_payload.clear();
					result.push(WsCompletedFrame::Other { raw });
				},
				Some(WebsocketFrameEvent::PayloadChunk { .. })
				| Some(WebsocketFrameEvent::Start { .. })
				| None => {},
			}
			remaining = &mut remaining[ret.consumed_bytes..];
		}
		Ok(result)
	}
}

// ---------------------------------------------------------------------------
// Guarded realtime proxy
// ---------------------------------------------------------------------------

/// A bidirectional guarded WebSocket proxy for the OpenAI Realtime API.
///
/// - **Client→Server:** applies request guards to `conversation.item.create` events.
/// - **Server→Client:** windowed evaluation of `response.output_text.delta` events —
///   deltas are held until ~`DEFAULT_EVAL_THRESHOLD` bytes of text accumulate, then
///   evaluated (with an overlap tail from previously evaluated text) and flushed on
///   pass. On block, the held (never-forwarded) deltas are discarded, a synthetic
///   error event is sent to the client, and `response.cancel` is sent to the server;
///   subsequent deltas for that response are dropped.
/// - Preserves existing telemetry on `response.done`.
/// - All non-text frames (audio, control, etc.) are forwarded immediately.
#[allow(clippy::too_many_arguments)]
pub async fn guarded_realtime_proxy<C, S>(
	client: C,
	server: S,
	frame_limit: usize,
	guard: PromptGuard,
	policy_client: PolicyClient,
	log: AsyncLog<LLMInfo>,
	req_headers: ::http::HeaderMap,
	original: Option<std::sync::Arc<crate::cel::RequestSnapshot>>,
	guardrail_log: crate::telemetry::log::GuardrailLog,
) where
	C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
	S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
	use crate::llm::policy::streaming_guardrails::{
		DEFAULT_EVAL_THRESHOLD, OVERLAP_BYTES, evaluate_window, tail_chars,
	};

	let (mut client_reader, mut client_writer_io) = tokio::io::split(client);
	let (mut server_reader, mut server_writer_io) = tokio::io::split(server);

	let (client_tx, mut client_rx) = tokio::sync::mpsc::channel::<Bytes>(256);
	let (server_tx, mut server_rx) = tokio::sync::mpsc::channel::<Bytes>(256);

	let guard_clone = guard.clone();
	let policy_client_clone = policy_client.clone();
	let request_guardrail_log = guardrail_log.clone();
	let log_clone = log.clone();
	let original_clone = original.clone();

	let client_to_server = {
		let server_tx = server_tx.clone();
		let client_tx_err = client_tx.clone();
		async move {
			let mut accum = WsFrameAccumulator::new(frame_limit);
			let mut read_buf = [0u8; 4096];

			loop {
				let n = match client_reader.read(&mut read_buf).await {
					Ok(0) | Err(_) => break,
					Ok(n) => n,
				};
				accum.push(&read_buf[..n]);

				for frame in accum.drain_frames()? {
					match frame {
						WsCompletedFrame::Text { raw, payload } => {
							if let Ok(text) = std::str::from_utf8(&payload)
								&& let Ok(RealtimeClientEvent::ConversationItemCreate(create)) =
									serde_json::from_str::<RealtimeClientEvent>(text)
							{
								let mut input_text = String::new();
								if let async_openai::types::realtime::RealtimeConversationItem::Message(
									async_openai::types::realtime::RealtimeConversationItemMessage::User(user_msg),
								) = &create.item
								{
									for content in &user_msg.content {
										if let UserMessageContent::InputText(t) = content {
											input_text.push_str(&t.text);
											input_text.push(' ');
										}
									}
								}

								if !input_text.is_empty()
									&& let Some(blocked_body) = guard
										.apply_realtime_request_guards(
											input_text.trim(),
											&policy_client,
											original.as_deref(),
											Some(&request_guardrail_log),
										)
										.await
								{
									let _ = client_tx_err
										.send(guardrail_blocked_ws_event_bytes(blocked_body))
										.await;
									continue;
								}
							}
							let _ = server_tx.send(raw).await;
						},
						WsCompletedFrame::Other { raw } => {
							let _ = server_tx.send(raw).await;
						},
					}
				}
			}
			Ok(())
		}
	};

	let server_to_client = {
		async move {
			let mut accum = WsFrameAccumulator::new(frame_limit);
			let mut seen_live_responses = HashSet::new();
			let mut read_buf = [0u8; 4096];
			let mut evaluators = guard_clone.begin_streaming_response_guard(
				&policy_client_clone,
				&req_headers,
				original_clone,
				guardrail_log,
			);
			let mut delta_hold: Vec<Bytes> = Vec::new();
			let mut pending_text = String::new();
			let mut overlap_tail = String::new();
			let mut response_blocked = false;

			loop {
				let n = match server_reader.read(&mut read_buf).await {
					Ok(0) | Err(_) => break,
					Ok(n) => n,
				};
				accum.push(&read_buf[..n]);

				for frame in accum.drain_frames()? {
					match frame {
						WsCompletedFrame::Text { raw, payload } => {
							if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&payload) {
								record_live_usage(&log_clone, &value, &mut seen_live_responses);
							}
							let event = std::str::from_utf8(&payload)
								.ok()
								.and_then(|s| serde_json::from_str::<RealtimeServerEvent>(s).ok());

							match event {
								Some(RealtimeServerEvent::ResponseOutputTextDelta(ref delta_event)) => {
									if response_blocked {
										continue;
									}
									pending_text.push_str(&delta_event.delta);
									delta_hold.push(raw);

									if pending_text.len() >= DEFAULT_EVAL_THRESHOLD {
										let batch = std::mem::take(&mut pending_text);
										let window = format!("{overlap_tail}{batch}");
										overlap_tail = tail_chars(&window, OVERLAP_BYTES).to_string();

										if let Some(blocked_body) = evaluate_window(&mut evaluators, &window).await {
											delta_hold.clear();
											// Clear text-state so a blocked response's content does not
											// bleed into the next response's evaluation window.
											overlap_tail.clear();
											pending_text.clear();
											response_blocked = true;
											let _ = client_tx
												.send(guardrail_blocked_ws_event_bytes(blocked_body))
												.await;
											let _ = server_tx
												.send(response_cancel_event_bytes([0, 0, 0, 0]))
												.await;
										} else {
											for f in delta_hold.drain(..) {
												let _ = client_tx.send(f).await;
											}
										}
									}
								},
								Some(RealtimeServerEvent::ResponseOutputTextDone(_)) => {
									if response_blocked {
										response_blocked = false;
										overlap_tail.clear();
										pending_text.clear();
										delta_hold.clear();
										continue;
									}

									let mut blocked_body = None;
									if !pending_text.is_empty() {
										let batch = std::mem::take(&mut pending_text);
										let window = format!("{overlap_tail}{batch}");
										blocked_body = evaluate_window(&mut evaluators, &window).await;
									}
									overlap_tail.clear();

									if let Some(blocked_body) = blocked_body {
										delta_hold.clear();
										response_blocked = true;
										let _ = client_tx
											.send(guardrail_blocked_ws_event_bytes(blocked_body))
											.await;
										let _ = server_tx
											.send(response_cancel_event_bytes([0, 0, 0, 0]))
											.await;
									} else {
										for f in delta_hold.drain(..) {
											let _ = client_tx.send(f).await;
										}
										let _ = client_tx.send(raw).await;
									}
								},
								Some(RealtimeServerEvent::ResponseDone(ref done_event)) => {
									if let Some(usage) = done_event.response.usage.as_ref() {
										if let Ok(usage) = serde_json::to_value(usage) {
											accumulate_realtime_usage(&log_clone, &usage);
										}
									}
									let _ = client_tx.send(raw).await;
								},
								_ => {
									let _ = client_tx.send(raw).await;
								},
							}
						},
						WsCompletedFrame::Other { raw } => {
							let _ = client_tx.send(raw).await;
						},
					}
				}
			}
			Ok(())
		}
	};

	// Spawn writer tasks as independent tokio tasks so they drain their channels
	// concurrently with the reader loops. Without independent tasks, the writers
	// would only run after a reader exits, which would drop buffered frames.
	let client_writer_join = tokio::spawn(async move {
		while let Some(bytes) = client_rx.recv().await {
			if client_writer_io.write_all(&bytes).await.is_err() {
				break;
			}
		}
	});

	let server_writer_join = tokio::spawn(async move {
		while let Some(bytes) = server_rx.recv().await {
			if server_writer_io.write_all(&bytes).await.is_err() {
				break;
			}
		}
	});

	// Exit when either reader closes; then wait for writers to drain remaining frames.
	let result: Result<(), FrameTooLarge> = tokio::select! {
		result = client_to_server => result,
		result = server_to_client => result,
	};
	if result.is_err() {
		client_writer_join.abort();
		server_writer_join.abort();
	}
	let _ = tokio::join!(client_writer_join, server_writer_join);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn realtime_usage_accumulates_across_responses_and_token_modalities() {
		let log: AsyncLog<LLMInfo> = AsyncLog::default();
		log.store(Some(LLMInfo::new(Default::default(), Default::default())));
		accumulate_realtime_usage(
			&log,
			&serde_json::json!({
				"input_tokens": 100,
				"output_tokens": 20,
				"total_tokens": 120,
				"input_token_details": {
					"text_tokens": 60,
					"audio_tokens": 40,
					"cached_tokens": 10
				},
				"output_token_details": {"text_tokens": 15, "audio_tokens": 5}
			}),
		);
		accumulate_realtime_usage(
			&log,
			&serde_json::json!({
				"input_tokens": 50,
				"output_tokens": 10,
				"total_tokens": 60,
				"input_token_details": {
					"text_tokens": 30,
					"audio_tokens": 20,
					"cached_tokens": 5
				},
				"output_token_details": {"text_tokens": 8, "audio_tokens": 2}
			}),
		);

		let response = log.load_clone().expect("log initialized").response;
		assert_eq!(response.input_tokens, Some(150));
		assert_eq!(response.output_tokens, Some(30));
		assert_eq!(response.total_tokens, Some(180));
		assert_eq!(response.input_text_tokens, Some(90));
		assert_eq!(response.input_audio_tokens, Some(60));
		assert_eq!(response.cached_input_tokens, Some(15));
		assert_eq!(response.output_text_tokens, Some(23));
		assert_eq!(response.output_audio_tokens, Some(7));
	}

	#[test]
	fn live_usage_replaces_duration_snapshots_and_deduplicates_completed_responses() {
		let log: AsyncLog<LLMInfo> = AsyncLog::default();
		log.store(Some(LLMInfo::new(Default::default(), Default::default())));
		let mut seen_responses = HashSet::new();
		for event in [
			serde_json::json!({
				"type": "session.started",
				"session": {"model": "gpt-live-1.1"}
			}),
			serde_json::json!({"type": "session.usage.updated", "usage": {"seconds": 12}}),
			serde_json::json!({"type": "session.usage.updated", "usage": {"seconds": 15}}),
		] {
			record_live_usage(&log, &event, &mut seen_responses);
		}

		let completion = serde_json::json!({
			"type": "response.event",
			"event": {
				"type": "response.completed",
				"response": {
					"id": "resp_1",
					"model": "gpt-4.1",
					"usage": {
						"input_tokens": 100,
						"output_tokens": 20,
						"total_tokens": 120,
					"input_tokens_details": {"cached_tokens": 10, "audio_tokens": 5},
					"output_tokens_details": {"audio_tokens": 7}
					}
				}
			}
		});
		record_live_usage(&log, &completion, &mut seen_responses);
		record_live_usage(&log, &completion, &mut seen_responses);
		record_live_usage(
			&log,
			&serde_json::json!({
				"type": "session.closed",
				"reason": "close_requested",
				"usage": {"seconds": 16}
			}),
			&mut seen_responses,
		);

		let response = log.load_clone().expect("log initialized").response;
		assert_eq!(response.provider_model.as_deref(), Some("gpt-live-1.1"));
		assert_eq!(response.audio_seconds, Some(16.0));
		assert_eq!(response.input_tokens, Some(100));
		assert_eq!(response.output_tokens, Some(20));
		assert_eq!(response.total_tokens, Some(120));
		assert_eq!(response.cached_input_tokens, Some(10));
		assert_eq!(response.usage_lines.len(), 1);
		assert_eq!(response.usage_lines[0].model.as_deref(), Some("gpt-4.1"));
		assert_eq!(response.usage_lines[0].input_audio_tokens, Some(5));
		assert_eq!(response.usage_lines[0].output_audio_tokens, Some(7));
		assert_eq!(response.finish_reasons.as_ref().map(|v| v[0].as_str()), Some("close_requested"));
	}

	#[test]
	fn checks_frame_limit_before_payload() {
		let mut frames = WsFrameAccumulator::new(4);
		let valid = encode_ws_text_frame_masked(b"test", [1, 2, 3, 4]);
		frames.push(&valid);
		let Ok(completed) = frames.drain_frames() else {
			panic!("frame at the limit should be accepted");
		};
		assert!(matches!(
			completed.as_slice(),
			[WsCompletedFrame::Text { raw, payload }]
				if raw == &valid && payload.as_ref() == b"test"
		));

		let header = WebsocketFrameEncoder::new().start_frame(&FrameInfo {
			opcode: Opcode::Text,
			payload_length: 5,
			mask: Some([1, 2, 3, 4]),
			fin: true,
			reserved: 0,
		});
		frames.push(&header);
		assert!(frames.drain_frames().is_err());
	}
}
