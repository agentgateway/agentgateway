use anyhow::anyhow;
use futures_core::Stream;
use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use rmcp::model::{RequestId, ServerJsonRpcMessage, ServerResult};
use std::sync::Arc;

use crate::mcp::ClientError;
use crate::mcp::streamablehttp::StreamableHttpPostResponse;
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
									.filter(|data| !data.is_empty())
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
pub type MessageMapper =
	Arc<dyn Fn(&str, ServerJsonRpcMessage) -> ServerJsonRpcMessage + Send + Sync>;

// Custom stream that merges multiple streams with terminal message handling
pub struct MergeStream {
	streams: Vec<Option<(Strng, Messages)>>,
	terminal_messages: Vec<Option<(Strng, ServerResult)>>,
	complete: bool,
	req_id: RequestId,
	merge: Option<Box<MergeFn>>,
	message_mapper: Option<MessageMapper>,
}

impl MergeStream {
	pub fn new_without_merge(
		streams: Vec<(Strng, Messages)>,
		message_mapper: Option<MessageMapper>,
	) -> Self {
		Self::new_internal(streams, RequestId::Number(0), None, message_mapper)
	}
	pub fn new(
		streams: Vec<(Strng, Messages)>,
		req_id: RequestId,
		merge: Box<MergeFn>,
		message_mapper: Option<MessageMapper>,
	) -> Self {
		Self::new_internal(streams, req_id, Some(merge), message_mapper)
	}
	fn new_internal(
		streams: Vec<(Strng, Messages)>,
		req_id: RequestId,
		merge: Option<Box<MergeFn>>,
		message_mapper: Option<MessageMapper>,
	) -> Self {
		let len = streams.len();
		let mut terminal_messages = Vec::with_capacity(len);
		terminal_messages.resize_with(len, || None);
		let mut wrapped_streams = Vec::with_capacity(len);
		wrapped_streams.extend(streams.into_iter().map(Some));
		Self {
			streams: wrapped_streams,
			terminal_messages,
			req_id,
			complete: false,
			merge,
			message_mapper,
		}
	}

	fn merge_terminal_messages(
		mut self: Pin<&mut Self>,
	) -> Result<ServerJsonRpcMessage, ClientError> {
		let mut msgs = Vec::with_capacity(self.terminal_messages.len());
		for terminal in &mut self.terminal_messages {
			if let Some(msg) = terminal.take() {
				msgs.push(msg);
			}
		}
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
			let polled = {
				let Some((_, msg_stream)) = self.streams[i].as_mut() else {
					continue;
				};
				msg_stream.0.as_mut().poll_next(cx)
			};

			match polled {
				Poll::Ready(Some(msg)) => match msg {
					Ok(ServerJsonRpcMessage::Response(r)) => {
						if let Some((server_name, _)) = self.streams[i].take() {
							self.terminal_messages[i] = Some((server_name, r.result));
						}
					},
					Ok(other) => {
						if let Some(mapper) = &self.message_mapper
							&& let Some((server_name, _)) = self.streams[i].as_ref()
						{
							return Poll::Ready(Some(Ok(mapper(server_name.as_str(), other))));
						}
						return Poll::Ready(Some(Ok(other)));
					},
					Err(e) => {
						self.complete = true;
						return Poll::Ready(Some(Err(e)));
					},
				},
				Poll::Ready(None) => {
					self.streams[i] = None;
				},
				Poll::Pending => {
					any_pending = true;
				},
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

#[cfg(test)]
mod tests {
	use super::*;
	use agent_core::strng::Strng;
	use futures_util::StreamExt;
	use rmcp::model::{
		CustomRequest, JsonRpcRequest, RequestId, ServerJsonRpcMessage, ServerRequest,
	};

	#[tokio::test]
	async fn maps_server_requests_for_downstream() {
		let req = JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(1),
			request: ServerRequest::CustomRequest(CustomRequest::new("elicitation/create", None)),
		};
		let messages =
			Messages(futures::stream::iter(vec![Ok(ServerJsonRpcMessage::Request(req))]).boxed());
		let mapper: MessageMapper = Arc::new(|server_name, message| match message {
			ServerJsonRpcMessage::Request(mut req) => {
				req.id = RequestId::String(format!("{server_name}-mapped").into());
				ServerJsonRpcMessage::Request(req)
			},
			other => other,
		});
		let mut merge =
			MergeStream::new_without_merge(vec![(Strng::from("upstream"), messages)], Some(mapper));
		let msg = merge.next().await.expect("message").expect("ok");
		match msg {
			ServerJsonRpcMessage::Request(req) => {
				assert_eq!(req.id, RequestId::String("upstream-mapped".into()));
			},
			other => panic!("unexpected message: {other:?}"),
		}
	}
}
