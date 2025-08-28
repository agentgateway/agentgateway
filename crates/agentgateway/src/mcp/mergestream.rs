use crate::mcp::relay::ClientError;
use crate::*;
use anyhow::anyhow;
use futures_core::Stream;
use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use itertools::Itertools;
use rmcp::model::{RequestId, ServerJsonRpcMessage, ServerResult};
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;

pub(crate) struct Messages(BoxStream<'static, Result<ServerJsonRpcMessage, ClientError>>);

impl TryFrom<StreamableHttpPostResponse> for Messages {
	type Error = ClientError;
	fn try_from(value: StreamableHttpPostResponse) -> Result<Self, Self::Error> {
		match value {
			StreamableHttpPostResponse::Accepted => {
				Err(ClientError::new(anyhow!("unexpected 'accepted' response")).into())
			},
			StreamableHttpPostResponse::Json(r, sid) => {
				Ok(Messages(futures::stream::once(async { Ok(r) }).boxed()))
			},
			StreamableHttpPostResponse::Sse(mut sse, sid) => Ok(Messages(
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
fn is_send_sync<T: Send + Sync>() {}
fn assert() {
	// is_send_sync::<MergeStream>();
}

// Custom stream that merges multiple streams with terminal message handling
pub struct MergeStream {
	streams: Vec<Option<Messages>>,
	terminal_messages: Vec<Option<ServerResult>>,
	complete: bool,
	merge:
		Box<dyn Fn(Vec<ServerResult>) -> Result<ServerResult, ClientError> + Send + Sync + 'static>,
}

impl MergeStream {
	pub fn new<F>(streams: Vec<Messages>, merge: F) -> Self
	where
		F: Fn(Vec<ServerResult>) -> Result<ServerResult, ClientError> + Send + Sync + 'static,
	{
		let terminal_messages = streams.iter().map(|s| None).collect::<Vec<_>>();
		Self {
			streams: streams.into_iter().map(Some).collect_vec(),
			terminal_messages,
			complete: false,
			merge: Box::new(merge),
		}
	}
}

impl MergeStream {
	fn merge_terminal_messages(
		mut self: Pin<&mut Self>,
	) -> Result<ServerJsonRpcMessage, ClientError> {
		let msgs = self
			.terminal_messages
			.iter_mut()
			.map(Option::take)
			.flatten()
			.collect_vec();
		let res = (self.merge)(msgs)?;
		Ok(ServerJsonRpcMessage::response(
			res.into(),
			RequestId::Number(1),
		))
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

		dbg!(
			&self
				.terminal_messages
				.iter()
				.map(|m| m.is_some())
				.collect::<Vec<_>>()
		);
		dbg!(&self.streams.iter().map(|m| m.is_some()).collect::<Vec<_>>());
		dbg!(&self.complete);
		for i in 0..self.streams.len() {
			tracing::error!("howardjohn: iter {i}");
			let res = {
				let mut msg_idx = self.streams[i].as_mut();
				let Some(msg_stream) = msg_idx else {
					tracing::error!("howardjohn: skip {i}");
					continue;
				};
				dbg!(msg_stream.0.as_mut().poll_next(cx))
			};

			let mut drop = false;
			match res {
				Poll::Ready(Some(msg)) => {
					match msg {
						Ok(ServerJsonRpcMessage::Response(r)) => {
							drop = true;
							self.terminal_messages[i] = Some(r.result);
							tracing::error!("howardjohn: set {i}");
							dbg!(
								&self
									.terminal_messages
									.iter()
									.map(|m| m.is_some())
									.collect::<Vec<_>>()
							);
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
					tracing::error!("howardjohn: end early");
				},
				Poll::Pending => {
					any_pending = true;
				},
			}
			if drop {
				self.streams[i] = None;
				tracing::error!("howardjohn: end early.. now={}", self.streams[i].is_some());
				// msg_idx.take();
			}

			tracing::error!("howardjohn: iter done {i}");
		}
		if any_pending {
			// Still waiting for some
			return Poll::Pending;
		}

		self.complete = true;
		Poll::Ready(Some(self.merge_terminal_messages()))
	}
}
