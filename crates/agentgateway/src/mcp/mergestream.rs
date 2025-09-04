use anyhow::anyhow;
use futures_core::Stream;
use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use itertools::Itertools;
use rmcp::model::{RequestId, ServerJsonRpcMessage, ServerResult};
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;

use crate::mcp::ClientError;
use crate::*;

pub(crate) struct Messages(BoxStream<'static, Result<ServerJsonRpcMessage, ClientError>>);

impl Messages {
	/// pending returns a stream that never returns any messages. It is not an empty stream that closes immediately; it hangs forever.
	pub fn pending() -> Self {
		Messages(futures::stream::pending().boxed())
	}
	/// empty returns a stream that never returns any messages. It immediately returns none.
	pub fn empty() -> Self {
		Messages(futures::stream::empty().boxed())
	}

	pub fn from_result<T: Into<ServerResult>>(id: RequestId, result: T) -> Self {
		Self::from(ServerJsonRpcMessage::response(result.into(), id))
	}
}

impl Stream for Messages {
	type Item = Result<ServerJsonRpcMessage, ClientError>;
	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		self.0.poll_next_unpin(cx)
	}
}

impl From<ServerJsonRpcMessage> for Messages {
	fn from(value: ServerJsonRpcMessage) -> Self {
		Messages(futures::stream::once(async { Ok(value) }).boxed())
	}
}

impl From<tokio::sync::mpsc::Receiver<ServerJsonRpcMessage>> for Messages {
	fn from(value: tokio::sync::mpsc::Receiver<ServerJsonRpcMessage>) -> Self {
		Messages(
			tokio_stream::wrappers::ReceiverStream::new(value)
				.map(Ok)
				.boxed(),
		)
	}
}

impl TryFrom<StreamableHttpPostResponse> for Messages {
	type Error = ClientError;
	fn try_from(value: StreamableHttpPostResponse) -> Result<Self, Self::Error> {
		match value {
			StreamableHttpPostResponse::Accepted => {
				Err(ClientError::new(anyhow!("unexpected 'accepted' response")))
			},
			StreamableHttpPostResponse::Json(r, _) => Ok(r.into()),
			StreamableHttpPostResponse::Sse(sse, _) => Ok(Messages(
				sse
					.filter_map(|item| async {
						item
							.map_err(ClientError::new)
							.and_then(|item| {
								item
									.data
									.map(|data| {
										serde_json::from_str::<ServerJsonRpcMessage>(&data).map_err(ClientError::new)
									})
									.transpose()
							})
							.transpose()
					})
					.boxed(),
			)),
		}
	}
}

pub type MergeFn = dyn FnOnce(Vec<(Strng, ServerResult)>) -> Result<ServerResult, ClientError>
	+ Send
	+ Sync
	+ 'static;

// Custom stream that merges multiple streams with terminal message handling
pub struct MergeStream {
	streams: Vec<Option<(Strng, Messages)>>,
	terminal_messages: Vec<Option<(Strng, ServerResult)>>,
	complete: bool,
	req_id: RequestId,
	merge: Option<Box<MergeFn>>,
}

impl MergeStream {
	pub fn new_without_merge(streams: Vec<(Strng, Messages)>) -> Self {
		Self::new_internal(streams, RequestId::Number(0), None)
	}
	pub fn new(streams: Vec<(Strng, Messages)>, req_id: RequestId, merge: Box<MergeFn>) -> Self {
		Self::new_internal(streams, req_id, Some(merge))
	}
	fn new_internal(
		streams: Vec<(Strng, Messages)>,
		req_id: RequestId,
		merge: Option<Box<MergeFn>>,
	) -> Self {
		let terminal_messages = streams.iter().map(|_| None).collect::<Vec<_>>();
		Self {
			streams: streams.into_iter().map(Some).collect_vec(),
			terminal_messages,
			req_id,
			complete: false,
			merge,
		}
	}

	fn merge_terminal_messages(
		mut self: Pin<&mut Self>,
	) -> Result<ServerJsonRpcMessage, ClientError> {
		let msgs = self
			.terminal_messages
			.iter_mut()
			.filter_map(Option::take)
			.collect_vec();
		let res = self
			.merge
			.take()
			.expect("merge_terminal_messages called twice")(msgs)?;
		Ok(ServerJsonRpcMessage::response(res, self.req_id.clone()))
	}
}

impl Stream for MergeStream {
	type Item = Result<ServerJsonRpcMessage, ClientError>;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		if self.complete {
			return Poll::Ready(None);
		}
		// Poll all active streams
		let mut any_pending = false;

		for i in 0..self.streams.len() {
			let (k, res) = {
				let msg_idx = self.streams[i].as_mut();
				let Some(msg_stream) = msg_idx else {
					continue;
				};
				(msg_stream.0.clone(), msg_stream.1.0.as_mut().poll_next(cx))
			};

			let mut drop = false;
			match res {
				Poll::Ready(Some(msg)) => {
					match msg {
						Ok(ServerJsonRpcMessage::Response(r)) => {
							drop = true;
							self.terminal_messages[i] = Some((k, r.result));
							// This stream is done, never look at it again
						},
						Err(e) => {
							self.complete = true;
							return Poll::Ready(Some(Err(e)));
						},
						_ => return Poll::Ready(Some(msg)),
					}
				},
				Poll::Ready(None) => {
					// Stream ended without terminal message (shouldn't happen in this design)
					// Not much we can do here I guess.
					drop = true;
				},
				Poll::Pending => {
					any_pending = true;
				},
			}
			if drop {
				self.streams[i] = None;
			}
		}
		if any_pending {
			// Still waiting for some
			return Poll::Pending;
		}

		self.complete = true;

		if self.merge.is_some() {
			Poll::Ready(Some(self.merge_terminal_messages()))
		} else {
			Poll::Ready(None)
		}
	}
}
