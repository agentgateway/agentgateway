use crate::llm::{LLMInfo, LLMRequest, LLMResponse};
use crate::parse::passthrough::PassthroughBody;
use crate::telemetry::log::AsyncLog;
use async_openai::types::realtime::Usage;
use bytes::{BufMut, BytesMut};
use jsonwebtoken::jwk::KeyOperations::DeriveBits;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio_stream::StreamExt;
use tokio_util::codec::Decoder;
use tungstenite::{Error, Message};

struct WebsocketParser<IO> {
	stream: tokio_tungstenite::WebSocketStream<IO>,
}

#[derive(Clone)]
struct Buf {
	buf: Arc<Mutex<BytesMut>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResponseDoneEvent {
	/// The response resource.
	pub response: ResponseResource,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResponseResource {
	/// Usage statistics for the response.
	pub usage: Option<Usage>,
}

pub async fn parser<IO>(
	body: IO,
	log: AsyncLog<LLMInfo>,
) -> impl AsyncRead + AsyncWrite + Unpin + 'static
where
	IO: AsyncRead + AsyncWrite + Unpin + 'static,
{
	// This is the server socket, so we want to *read* the incoming data
	let (rh, wh) = tokio::io::split(body);
	let (rb, mut wb) = tokio::io::simplex(100_000);
	let nr = tokio_util::io::InspectReader::new(rh, move |d| {
		let waker = futures::task::noop_waker();
		let mut cx = Context::from_waker(&waker);
		if Pin::new(&mut wb).poll_write(&mut cx, d).is_pending() {
			panic!("TODO I thought we couldn't be pending??");
		}
	});
	let js = tokio::io::join(rb, tokio::io::sink());
	let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
		js,
		tungstenite::protocol::Role::Client,
		None,
	)
	.await;
	tokio::task::spawn(async move {
		while let Some(msg) = ws.next().await {
			match msg {
				Ok(t) => match t {
					Message::Text(b) => {
						// tracing::error!("howardjohn: {}", b.as_str());
						if b.contains("response.done") {
							let Ok(typed) = serde_json::from_str::<ResponseDoneEvent>(b.as_str()) else {
								continue;
							};
							if let Some(usage) = typed.response.usage {
								// TODO: do we need to parse the request side to get the request model?
								// it seems like we get an event from the server with the same thing.
								// also, the model can change... so what do we report??
								log.store(Some(LLMInfo {
									request: LLMRequest {
										input_tokens: None,
										input_format: crate::llm::InputFormat::Realtime,
										request_model: Default::default(), // TODO
										provider: Default::default(), // TODO
										streaming: true,
										params: Default::default(),
									},
									response: LLMResponse {
										input_tokens: Some(usage.input_tokens as u64),
										output_tokens: Some(usage.output_tokens as u64),
										total_tokens: Some(usage.total_tokens as u64),
										provider_model: None,
										completion: None,
										first_token: None,
									},
								}));
								tracing::error!("howardjohn: {:?}", usage);
							}
						}
					},
					Message::Binary(_) => {},
					Message::Ping(_) => {},
					Message::Pong(_) => {},
					Message::Close(_) => {},
					Message::Frame(_) => {},
				},
				Err(e) => {
					tracing::error!("howardjohn: error: {e}");
				},
			}
		}
	});
	tokio::io::join(nr, wh)
	//   let x = ws.next().await.unwrap().unwrap();
	//   x.
}

impl<IO> AsyncRead for WebsocketParser<IO> {
	fn poll_read(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		todo!()
	}
}
