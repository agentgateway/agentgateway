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
use crate::llm::{LLMInfo, LLMResponse};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::AsyncLog;

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
	decoder: websocket_sans_io::WebsocketFrameDecoder,
	buf: BytesMut,
	buffer_limit: usize,
	disabled: bool,
	log: AsyncLog<LLMInfo>,
}

impl<IO> Parser<IO> {
	fn record_text_payload(&mut self, data: &[u8]) -> bool {
		if self.buf.len() + data.len() > self.buffer_limit {
			self.buf = Default::default();
			self.disabled = true;
			return false;
		}
		self.buf.extend_from_slice(data);
		true
	}

	fn emit(&self, data: Bytes) {
		let Ok(data) = str::from_utf8(&data) else {
			return;
		};
		if data.contains("response.done") {
			let Ok(typed) = serde_json::from_str::<ResponseDoneEvent>(data) else {
				return;
			};
			if let Some(usage) = typed.response.usage {
				// TODO: do we need to parse the request side to get the request model?
				// it seems like we get an event from the server with the same thing.
				// also, the model can change... so what do we report??
				self.log.non_atomic_mutate(|r| {
					r.response = LLMResponse {
						input_tokens: Some(usage.input_tokens as u64),
						input_image_tokens: None,
						input_text_tokens: None,
						input_audio_tokens: None,
						output_tokens: Some(usage.output_tokens as u64),
						output_image_tokens: None,
						output_text_tokens: None,
						output_audio_tokens: None,
						total_tokens: Some(usage.total_tokens as u64),
						service_tier: None,
						provider_model: None,
						completion: None,
						first_token: None,
						count_tokens: None,
						reasoning_tokens: None,
						cache_creation_input_tokens: None,
						cached_input_tokens: usage
							.input_token_details
							.as_ref()
							.and_then(|d| d.cached_tokens)
							.map(|x| x as u64),
					}
				});
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
		if self.disabled {
			return Poll::Ready(Ok(()));
		}
		let mut processed_offset = 0;
		loop {
			let unprocessed_part_of_buf = &buf.filled()[processed_offset..buf.filled().len()];
			// Websocket logic needs owned copy to apply the mask. However, we need to keep the untouched stuff
			// so we are not modifying the response.
			let Ok(ret) = self.decoder.add_data(&mut unprocessed_part_of_buf.to_vec());
			processed_offset += ret.consumed_bytes;

			if ret.event.is_none() && ret.consumed_bytes == 0 {
				return Poll::Ready(Ok(()));
			}

			match ret.event {
				Some(WebsocketFrameEvent::PayloadChunk {
					original_opcode: Opcode::Text,
				}) if !self.record_text_payload(&unprocessed_part_of_buf[0..ret.consumed_bytes]) => {
					return Poll::Ready(Ok(()));
				},
				Some(WebsocketFrameEvent::End {
					frame_info: FrameInfo { fin: true, .. },
					original_opcode: Opcode::Text,
				}) => {
					let got = self.buf.split();
					self.emit(got.freeze());
				},
				_ => (),
			}
		}
	}
}

pub async fn parser<IO>(
	body: IO,
	log: AsyncLog<LLMInfo>,
) -> impl AsyncRead + AsyncWrite + Unpin + 'static
where
	IO: AsyncRead + AsyncWrite + Unpin + 'static,
{
	parser_with_limit(body, log, crate::defaults::max_buffer_size())
}

fn parser_with_limit<IO>(
	body: IO,
	log: AsyncLog<LLMInfo>,
	buffer_limit: usize,
) -> impl AsyncRead + AsyncWrite + Unpin + 'static
where
	IO: AsyncRead + AsyncWrite + Unpin + 'static,
{
	Parser {
		inner: body,
		decoder: websocket_sans_io::WebsocketFrameDecoder::new(),
		buf: Default::default(),
		buffer_limit,
		disabled: false,
		log,
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
fn guardrail_blocked_ws_event_bytes() -> Bytes {
	let json = br#"{"type":"error","error":{"type":"guardrail_blocked","code":"guardrail_intervention","message":"Content blocked by guardrail policy"}}"#;
	encode_ws_text_frame(json)
}

/// Synthetic `response.cancel` event sent to the server when a response guard blocks.
///
/// The WebSocket spec requires client→server frames to be masked. We use a zero mask
/// so the XOR is identity (payload bytes are unchanged) while the frame is formally masked.
fn response_cancel_event_bytes(mask: [u8; 4]) -> Bytes {
	let json = br#"{"type":"response.cancel"}"#;
	encode_ws_text_frame_masked(json, mask)
}

// ---------------------------------------------------------------------------
// WebSocket frame accumulator
// ---------------------------------------------------------------------------

struct WsFrameAccumulator {
	decoder: websocket_sans_io::WebsocketFrameDecoder,
	pending: BytesMut,
	frame_raw: BytesMut,
	frame_payload: BytesMut,
}

enum WsCompletedFrame {
	Text { raw: Bytes, payload: Bytes },
	Other { raw: Bytes },
}

impl WsFrameAccumulator {
	fn new() -> Self {
		Self {
			decoder: websocket_sans_io::WebsocketFrameDecoder::new(),
			pending: BytesMut::new(),
			frame_raw: BytesMut::new(),
			frame_payload: BytesMut::new(),
		}
	}

	fn push(&mut self, data: &[u8]) {
		self.pending.extend_from_slice(data);
	}

	fn drain_frames(&mut self) -> Vec<WsCompletedFrame> {
		let mut result = Vec::new();
		loop {
			let mut copy = self.pending.to_vec();
			let ret = match self.decoder.add_data(&mut copy) {
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

			let raw_chunk = self.pending.split_to(ret.consumed_bytes).freeze();
			self.frame_raw.extend_from_slice(&raw_chunk);

			match ret.event {
				Some(WebsocketFrameEvent::PayloadChunk {
					original_opcode: Opcode::Text,
				}) => {
					self
						.frame_payload
						.extend_from_slice(&copy[..ret.consumed_bytes]);
				},
				Some(WebsocketFrameEvent::End {
					original_opcode: Opcode::Text,
					frame_info: FrameInfo { fin: true, .. },
				}) => {
					self
						.frame_payload
						.extend_from_slice(&copy[..ret.consumed_bytes]);
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
		}
		result
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
pub async fn guarded_realtime_proxy<C, S>(
	client: C,
	server: S,
	guard: PromptGuard,
	policy_client: PolicyClient,
	log: AsyncLog<LLMInfo>,
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
	let log_clone = log.clone();

	let client_to_server = {
		let server_tx = server_tx.clone();
		let client_tx_err = client_tx.clone();
		async move {
			let mut accum = WsFrameAccumulator::new();
			let mut read_buf = [0u8; 4096];

			loop {
				let n = match client_reader.read(&mut read_buf).await {
					Ok(0) | Err(_) => break,
					Ok(n) => n,
				};
				accum.push(&read_buf[..n]);

				for frame in accum.drain_frames() {
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
									&& guard
										.apply_realtime_request_guards(input_text.trim(), &policy_client)
										.await
								{
									let _ = client_tx_err.send(guardrail_blocked_ws_event_bytes()).await;
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
			drop(server_tx);
		}
	};

	let server_to_client = {
		async move {
			let mut accum = WsFrameAccumulator::new();
			let mut read_buf = [0u8; 4096];
			let mut evaluators =
				guard_clone.begin_streaming_response_guard(&policy_client_clone, &::http::HeaderMap::new());
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

				for frame in accum.drain_frames() {
					match frame {
						WsCompletedFrame::Text { raw, payload } => {
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

										if evaluate_window(&mut evaluators, &window).await {
											delta_hold.clear();
											response_blocked = true;
											let _ = client_tx.send(guardrail_blocked_ws_event_bytes()).await;
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

									let mut blocked = false;
									if !pending_text.is_empty() {
										let batch = std::mem::take(&mut pending_text);
										let window = format!("{overlap_tail}{batch}");
										blocked = evaluate_window(&mut evaluators, &window).await;
									}
									overlap_tail.clear();

									if blocked {
										delta_hold.clear();
										// Prevent stale deltas from a race between the cancel
										// reaching the server and buffered frames arriving here.
										response_blocked = true;
										let _ = client_tx.send(guardrail_blocked_ws_event_bytes()).await;
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
										let usage_clone = usage.clone();
										log_clone.non_atomic_mutate(|r| {
											r.response = LLMResponse {
												input_tokens: Some(usage_clone.input_tokens as u64),
												input_image_tokens: None,
												input_text_tokens: None,
												input_audio_tokens: None,
												output_tokens: Some(usage_clone.output_tokens as u64),
												output_image_tokens: None,
												output_text_tokens: None,
												output_audio_tokens: None,
												total_tokens: Some(usage_clone.total_tokens as u64),
												service_tier: None,
												provider_model: None,
												completion: None,
												first_token: None,
												count_tokens: None,
												reasoning_tokens: None,
												cache_creation_input_tokens: None,
												cached_input_tokens: usage_clone
													.input_token_details
													.as_ref()
													.and_then(|d| d.cached_tokens)
													.map(|x| x as u64),
											}
										});
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
			drop(client_tx);
		}
	};

	let client_writer_task = async move {
		while let Some(bytes) = client_rx.recv().await {
			if client_writer_io.write_all(&bytes).await.is_err() {
				break;
			}
		}
	};

	let server_writer_task = async move {
		while let Some(bytes) = server_rx.recv().await {
			if server_writer_io.write_all(&bytes).await.is_err() {
				break;
			}
		}
	};

	tokio::select! {
		_ = client_to_server => {},
		_ = server_to_client => {},
		_ = client_writer_task => {},
		_ = server_writer_task => {},
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn record_text_payload_disables_after_limit() {
		let (io, _) = tokio::io::duplex(1);
		let mut parser = Parser {
			inner: io,
			decoder: websocket_sans_io::WebsocketFrameDecoder::new(),
			buf: Default::default(),
			buffer_limit: 4,
			disabled: false,
			log: AsyncLog::default(),
		};

		assert!(parser.record_text_payload(b"abc"));
		assert_eq!(&parser.buf[..], b"abc");
		parser.buf.reserve(1024);
		assert!(!parser.record_text_payload(b"de"));
		assert!(parser.disabled);
		assert!(parser.buf.is_empty());
		assert_eq!(parser.buf.capacity(), 0);
	}
}
