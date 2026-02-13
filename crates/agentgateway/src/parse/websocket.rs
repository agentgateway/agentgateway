use std::io::{Error, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use async_openai::types::realtime::RealtimeResponseUsage;
use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use websocket_sans_io::{FrameInfo, Opcode, WebsocketFrameEvent};

use crate::llm::{LLMInfo, LLMResponse};
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
	log: AsyncLog<LLMInfo>,
}

impl<IO> Parser<IO> {
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
						output_tokens: Some(usage.output_tokens as u64),
						total_tokens: Some(usage.total_tokens as u64),
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
				}) => {
					self
						.buf
						.extend_from_slice(&unprocessed_part_of_buf[0..ret.consumed_bytes]);
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
	Parser {
		inner: body,
		decoder: websocket_sans_io::WebsocketFrameDecoder::new(),
		buf: Default::default(),
		log,
	}
}
