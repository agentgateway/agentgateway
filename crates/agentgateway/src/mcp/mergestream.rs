use anyhow::anyhow;
use futures_core::Stream;
use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use itertools::Itertools;
use rmcp::model::{RequestId, ServerJsonRpcMessage, ServerResult};
use tracing::warn;

use crate::mcp::rbac::CelExecWrapper;
use crate::mcp::streamablehttp::StreamableHttpPostResponse;
use crate::mcp::{ClientError, FailureMode};
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

	pub fn then_pending(self) -> Self {
		Messages(self.0.chain(futures::stream::pending()).boxed())
	}

	pub fn from_result<T: Into<ServerResult>>(id: RequestId, result: T) -> Self {
		Self::from(ServerJsonRpcMessage::response(result.into(), id))
	}

	pub fn map_server_messages(
		self,
		mut f: impl FnMut(ServerJsonRpcMessage) -> ServerJsonRpcMessage + Send + 'static,
	) -> Self {
		Messages(
			self
				.0
				.map(move |message| match message {
					Ok(message) => Ok(f(message)),
					Err(err) => Err(err),
				})
				.boxed(),
		)
	}

	/// One-pass filter+rewrite+tag where the mapping fn may drop a message (`None`)
	/// or turn an `Ok` message into an `Err`.
	pub fn filter_map_messages_result(
		self,
		mut f: impl FnMut(ServerJsonRpcMessage) -> Option<Result<ServerJsonRpcMessage, ClientError>>
		+ Send
		+ 'static,
	) -> Self {
		Messages(
			self
				.0
				.filter_map(move |message| {
					let mapped = match message {
						Ok(message) => f(message),
						Err(err) => Some(Err(err)),
					};
					async move { mapped }
				})
				.boxed(),
		)
	}
}

#[cfg(test)]
impl Messages {
	/// Build a `Messages` from a fixed list of results, for driving pipeline tests.
	pub(crate) fn from_results(items: Vec<Result<ServerJsonRpcMessage, ClientError>>) -> Self {
		Messages(futures::stream::iter(items).boxed())
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

impl From<Result<ServerJsonRpcMessage, ClientError>> for Messages {
	fn from(value: Result<ServerJsonRpcMessage, ClientError>) -> Self {
		Messages(futures::stream::once(async { value }).boxed())
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

pub type MergeFn = dyn FnOnce(
		Vec<(Strng, ServerResult)>,
		// Request CEL context, shared across all upstreams.
		&CelExecWrapper,
	) -> Result<ServerResult, ClientError>
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
	// Present iff `merge` is; supplied to the merge fn for RBAC filtering.
	cel: Option<CelExecWrapper>,
	failure_mode: FailureMode,
}

impl MergeStream {
	pub fn new_without_merge(streams: Vec<(Strng, Messages)>, failure_mode: FailureMode) -> Self {
		Self::new_internal(streams, RequestId::Number(0), None, None, failure_mode)
	}
	pub fn new(
		streams: Vec<(Strng, Messages)>,
		req_id: RequestId,
		merge: Box<MergeFn>,
		cel: CelExecWrapper,
		failure_mode: FailureMode,
	) -> Self {
		Self::new_internal(streams, req_id, Some(merge), Some(cel), failure_mode)
	}
	fn new_internal(
		streams: Vec<(Strng, Messages)>,
		req_id: RequestId,
		merge: Option<Box<MergeFn>>,
		cel: Option<CelExecWrapper>,
		failure_mode: FailureMode,
	) -> Self {
		let terminal_messages = streams.iter().map(|_| None).collect::<Vec<_>>();
		Self {
			streams: streams.into_iter().map(Some).collect_vec(),
			terminal_messages,
			req_id,
			complete: false,
			merge,
			cel,
			failure_mode,
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

		let merge = self
			.merge
			.take()
			.expect("merge_terminal_messages called twice");
		let cel = self.cel.as_ref().expect("merge is present iff cel is");
		let res = merge(msgs, cel)?;
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
						Ok(ServerJsonRpcMessage::Error(e)) => {
							if self.failure_mode == FailureMode::FailOpen {
								warn!(
									"upstream JSON-RPC error, skipping (failure_mode=FailOpen): {:?}",
									e
								);
								drop = true;
							} else {
								self.complete = true;
								return Poll::Ready(Some(Ok(ServerJsonRpcMessage::Error(e))));
							}
						},
						Err(e) => {
							if self.failure_mode == FailureMode::FailOpen {
								warn!(
									"upstream stream error, skipping (failure_mode=FailOpen): {}",
									e
								);
								drop = true;
							} else {
								self.complete = true;
								return Poll::Ready(Some(Err(e)));
							}
						},
						_ => return Poll::Ready(Some(msg)),
					}
				},
				Poll::Ready(None) => {
					// Long-lived streams can end without a terminal response.
					if self.failure_mode == FailureMode::FailOpen {
						warn!("upstream stream ended unexpectedly, skipping (failure_mode=FailOpen)");
						drop = true;
					} else {
						self.complete = true;
						return Poll::Ready(Some(Err(ClientError::new(anyhow::anyhow!(
							"upstream stream ended unexpectedly"
						)))));
					}
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
