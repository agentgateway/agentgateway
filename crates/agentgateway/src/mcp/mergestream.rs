use anyhow::anyhow;
use futures_core::Stream;
use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use rmcp::model::{JsonRpcError, RequestId, ServerJsonRpcMessage, ServerResult};
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

	pub fn from_stream<S>(stream: S) -> Self
	where
		S: Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
	{
		Messages(stream.boxed())
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
pub type MessageMapper = Arc<
	dyn Fn(&str, ServerJsonRpcMessage) -> Result<ServerJsonRpcMessage, ClientError> + Send + Sync,
>;

type TerminalSuccess = (Strng, ServerResult);
type TerminalError = (Strng, JsonRpcError);

enum TerminalOutcome {
	Success(Box<TerminalSuccess>),
	Error(TerminalError),
}

// Custom stream that merges multiple streams with terminal message handling
pub struct MergeStream {
	streams: Vec<Option<(Strng, Messages)>>,
	terminal_outcomes: Vec<Option<TerminalOutcome>>,
	stream_errors: Vec<Option<ClientError>>,
	complete: bool,
	next_poll_start: usize,
	fail_on_stream_error: bool,
	req_id: RequestId,
	merge: Option<Box<MergeFn>>,
	message_mapper: Option<MessageMapper>,
}

impl MergeStream {
	pub fn new_without_merge(
		streams: Vec<(Strng, Messages)>,
		message_mapper: Option<MessageMapper>,
	) -> Self {
		Self::new_internal(streams, RequestId::Number(0), None, false, message_mapper)
	}
	pub fn new(
		streams: Vec<(Strng, Messages)>,
		req_id: RequestId,
		merge: Box<MergeFn>,
		fail_on_stream_error: bool,
		message_mapper: Option<MessageMapper>,
	) -> Self {
		Self::new_internal(
			streams,
			req_id,
			Some(merge),
			fail_on_stream_error,
			message_mapper,
		)
	}
	fn new_internal(
		streams: Vec<(Strng, Messages)>,
		req_id: RequestId,
		merge: Option<Box<MergeFn>>,
		fail_on_stream_error: bool,
		message_mapper: Option<MessageMapper>,
	) -> Self {
		let len = streams.len();
		let mut terminal_outcomes = Vec::with_capacity(len);
		terminal_outcomes.resize_with(len, || None);
		let mut stream_errors = Vec::with_capacity(len);
		stream_errors.resize_with(len, || None);
		let mut wrapped_streams = Vec::with_capacity(len);
		wrapped_streams.extend(streams.into_iter().map(Some));
		Self {
			streams: wrapped_streams,
			terminal_outcomes,
			stream_errors,
			req_id,
			complete: false,
			next_poll_start: 0,
			fail_on_stream_error,
			merge,
			message_mapper,
		}
	}

	fn take_stream_errors(&mut self) -> Vec<ClientError> {
		self
			.stream_errors
			.iter_mut()
			.filter_map(Option::take)
			.collect()
	}

	fn take_terminal_outcomes(&mut self) -> (Vec<TerminalSuccess>, Vec<TerminalError>) {
		let mut successes = Vec::with_capacity(self.terminal_outcomes.len());
		let mut terminal_errors = Vec::new();
		for outcome in &mut self.terminal_outcomes {
			match outcome.take() {
				Some(TerminalOutcome::Success(success)) => successes.push(*success),
				Some(TerminalOutcome::Error(error)) => terminal_errors.push(error),
				None => {},
			}
		}
		(successes, terminal_errors)
	}

	fn terminal_error_message(
		terminal_errors: &[TerminalError],
		stream_errors: &[ClientError],
	) -> String {
		let mut failures = terminal_errors
			.iter()
			.map(|(server_name, err)| {
				format!(
					"{server_name}: upstream returned JSON-RPC error {:?}: {}",
					err.error.code, err.error.message
				)
			})
			.collect::<Vec<_>>();
		failures.extend(stream_errors.iter().map(ToString::to_string));
		failures.join("; ")
	}

	fn merge_terminal_outcomes(
		mut self: Pin<&mut Self>,
		mut successes: Vec<TerminalSuccess>,
		terminal_errors: Vec<TerminalError>,
		mut stream_errors: Vec<ClientError>,
	) -> Result<ServerJsonRpcMessage, ClientError> {
		// In merged fanout, the gateway has to produce one aggregate terminal outcome.
		// Strict mode therefore treats any upstream terminal failure as fatal for the
		// merged request instead of forwarding a raw upstream error frame and continuing.
		if self.fail_on_stream_error && (!terminal_errors.is_empty() || !stream_errors.is_empty()) {
			let message = Self::terminal_error_message(&terminal_errors, &stream_errors);
			return Err(ClientError::new(anyhow!(
				"one or more upstreams failed during merge: {message}",
			)));
		}
		if successes.is_empty() {
			if !terminal_errors.is_empty() {
				let message = Self::terminal_error_message(&terminal_errors, &stream_errors);
				return Err(ClientError::new(anyhow!(
					"all upstreams terminated with errors: {message}",
				)));
			}
			let err = match stream_errors.len() {
				0 => ClientError::new(anyhow!(
					"all upstream streams ended before terminal response",
				)),
				1 => stream_errors.remove(0),
				_ => {
					let message = stream_errors
						.into_iter()
						.map(|e| e.to_string())
						.collect::<Vec<_>>()
						.join("; ");
					ClientError::new(anyhow!(
						"all upstream streams failed before terminal response: {message}",
					))
				},
			};
			return Err(err);
		}
		let res = self
			.merge
			.take()
			.expect("merge_terminal_outcomes called twice")(std::mem::take(&mut successes))?;
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
		let len = self.streams.len();

		for offset in 0..len {
			let i = (self.next_poll_start + offset) % len;
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
							self.terminal_outcomes[i] =
								Some(TerminalOutcome::Success(Box::new((server_name, r.result))));
						}
					},
					Ok(ServerJsonRpcMessage::Error(err)) => {
						if let Some((server_name, _)) = self.streams[i].take() {
							// For merged fanout, an upstream JSON-RPC error is that upstream's
							// final request outcome, so it must stop counting as pending.
							self.terminal_outcomes[i] = Some(TerminalOutcome::Error((server_name, err)));
						}
					},
					Ok(other) => {
						if let Some(mapper) = &self.message_mapper {
							let mapped = if let Some((server_name, _)) = self.streams[i].as_ref() {
								mapper(server_name.as_str(), other)
							} else {
								Ok(other)
							};
							match mapped {
								Ok(message) => {
									self.next_poll_start = (i + 1) % len;
									return Poll::Ready(Some(Ok(message)));
								},
								Err(error) => {
									if let Some((server_name, _)) = self.streams[i].take() {
										tracing::warn!(
											%server_name,
											error = %error,
											"gateway failed to map merged upstream message; continuing with remaining upstreams"
										);
									}
									self.stream_errors[i] = Some(error);
									continue;
								},
							}
						}
						self.next_poll_start = (i + 1) % len;
						return Poll::Ready(Some(Ok(other)));
					},
					Err(e) => {
						if let Some((server_name, _)) = self.streams[i].take() {
							tracing::warn!(
								%server_name,
								error = %e,
								"upstream stream failed during merge; continuing with remaining upstreams"
							);
						}
						self.stream_errors[i] = Some(e);
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
			let stream_errors = self.take_stream_errors();
			let (successes, terminal_errors) = self.take_terminal_outcomes();
			Poll::Ready(Some(self.merge_terminal_outcomes(
				successes,
				terminal_errors,
				stream_errors,
			)))
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
		let mapper: MessageMapper = Arc::new(|server_name, message| {
			Ok(match message {
				ServerJsonRpcMessage::Request(mut req) => {
					req.id = RequestId::String(format!("{server_name}-mapped").into());
					ServerJsonRpcMessage::Request(req)
				},
				other => other,
			})
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

	#[tokio::test]
	async fn continues_when_one_upstream_errors_during_merge() {
		let failing =
			Messages(futures::stream::iter(vec![Err(ClientError::new(anyhow!("boom")))]).boxed());
		let healthy = Messages(
			futures::stream::iter(vec![Ok(ServerJsonRpcMessage::response(
				ServerResult::empty(()),
				RequestId::Number(1),
			))])
			.boxed(),
		);

		let mut merge = MergeStream::new(
			vec![(Strng::from("bad"), failing), (Strng::from("ok"), healthy)],
			RequestId::Number(99),
			Box::new(|streams| {
				assert_eq!(streams.len(), 1);
				assert_eq!(streams[0].0.as_str(), "ok");
				Ok(ServerResult::empty(()))
			}),
			false,
			None,
		);

		let msg = merge.next().await.expect("message").expect("ok");
		assert!(matches!(msg, ServerJsonRpcMessage::Response(_)));
	}

	#[tokio::test]
	async fn returns_error_when_all_upstreams_error_before_terminal_response() {
		let s1 =
			Messages(futures::stream::iter(vec![Err(ClientError::new(anyhow!("boom-1")))]).boxed());
		let s2 =
			Messages(futures::stream::iter(vec![Err(ClientError::new(anyhow!("boom-2")))]).boxed());

		let mut merge = MergeStream::new(
			vec![(Strng::from("s1"), s1), (Strng::from("s2"), s2)],
			RequestId::Number(100),
			Box::new(|_| Ok(ServerResult::empty(()))),
			false,
			None,
		);

		let err = merge
			.next()
			.await
			.expect("message")
			.expect_err("expected stream error");
		let msg = err.to_string();
		assert!(
			msg.contains("boom-1") && msg.contains("boom-2"),
			"unexpected error: {msg}"
		);
	}

	#[tokio::test]
	async fn rotates_first_ready_stream_across_polls() {
		let s1 = Messages(
			futures::stream::iter(vec![
				Ok(ServerJsonRpcMessage::notification(
					rmcp::model::ServerNotification::CustomNotification(
						rmcp::model::CustomNotification::new("input-a", None),
					),
				)),
				Ok(ServerJsonRpcMessage::notification(
					rmcp::model::ServerNotification::CustomNotification(
						rmcp::model::CustomNotification::new("input-b", None),
					),
				)),
			])
			.boxed(),
		);
		let s2 = Messages(
			futures::stream::iter(vec![Ok(ServerJsonRpcMessage::notification(
				rmcp::model::ServerNotification::CustomNotification(rmcp::model::CustomNotification::new(
					"input-c", None,
				)),
			))])
			.boxed(),
		);
		let mapper: MessageMapper = Arc::new(|server_name, message| match message {
			ServerJsonRpcMessage::Notification(_) => Ok(ServerJsonRpcMessage::notification(
				rmcp::model::ServerNotification::CustomNotification(rmcp::model::CustomNotification::new(
					server_name,
					None,
				)),
			)),
			other => Ok(other),
		});

		let mut merge = MergeStream::new_without_merge(
			vec![(Strng::from("s1"), s1), (Strng::from("s2"), s2)],
			Some(mapper),
		);

		let first = merge.next().await.expect("first message").expect("ok");
		let second = merge.next().await.expect("second message").expect("ok");

		let extract_method = |message: ServerJsonRpcMessage| match message {
			ServerJsonRpcMessage::Notification(notification) => match notification.notification {
				rmcp::model::ServerNotification::CustomNotification(notification) => {
					notification.method.to_string()
				},
				other => panic!("unexpected notification: {other:?}"),
			},
			other => panic!("unexpected message: {other:?}"),
		};

		assert_eq!(extract_method(first), "s1");
		assert_eq!(extract_method(second), "s2");
	}

	#[tokio::test]
	async fn fails_merge_when_strict_and_any_upstream_errors() {
		let failing =
			Messages(futures::stream::iter(vec![Err(ClientError::new(anyhow!("boom")))]).boxed());
		let healthy = Messages(
			futures::stream::iter(vec![Ok(ServerJsonRpcMessage::response(
				ServerResult::empty(()),
				RequestId::Number(1),
			))])
			.boxed(),
		);

		let mut merge = MergeStream::new(
			vec![(Strng::from("bad"), failing), (Strng::from("ok"), healthy)],
			RequestId::Number(99),
			Box::new(|_| Ok(ServerResult::empty(()))),
			true,
			None,
		);

		let err = merge
			.next()
			.await
			.expect("message")
			.expect_err("should fail");
		assert!(
			err
				.to_string()
				.contains("one or more upstreams failed during merge")
		);
	}

	#[tokio::test]
	async fn continues_when_one_upstream_returns_terminal_error_in_degraded_mode() {
		let failing = Messages(
			futures::stream::iter(vec![Ok(ServerJsonRpcMessage::error(
				rmcp::ErrorData::invalid_request("boom".to_string(), None),
				RequestId::Number(1),
			))])
			.boxed(),
		);
		let healthy = Messages(
			futures::stream::iter(vec![Ok(ServerJsonRpcMessage::response(
				ServerResult::empty(()),
				RequestId::Number(2),
			))])
			.boxed(),
		);

		let mut merge = MergeStream::new(
			vec![(Strng::from("bad"), failing), (Strng::from("ok"), healthy)],
			RequestId::Number(99),
			Box::new(|streams| {
				assert_eq!(streams.len(), 1);
				assert_eq!(streams[0].0.as_str(), "ok");
				Ok(ServerResult::empty(()))
			}),
			false,
			None,
		);

		let msg = merge.next().await.expect("message").expect("ok");
		assert!(
			matches!(msg, ServerJsonRpcMessage::Response(_)),
			"terminal upstream error should not be emitted before the aggregate outcome"
		);
	}

	#[tokio::test]
	async fn returns_error_when_all_upstreams_terminate_with_terminal_errors() {
		let s1 = Messages(
			futures::stream::iter(vec![Ok(ServerJsonRpcMessage::error(
				rmcp::ErrorData::invalid_request("boom-1".to_string(), None),
				RequestId::Number(1),
			))])
			.boxed(),
		);
		let s2 = Messages(
			futures::stream::iter(vec![Ok(ServerJsonRpcMessage::error(
				rmcp::ErrorData::invalid_request("boom-2".to_string(), None),
				RequestId::Number(2),
			))])
			.boxed(),
		);

		let mut merge = MergeStream::new(
			vec![(Strng::from("s1"), s1), (Strng::from("s2"), s2)],
			RequestId::Number(100),
			Box::new(|_| Ok(ServerResult::empty(()))),
			false,
			None,
		);

		let err = merge
			.next()
			.await
			.expect("message")
			.expect_err("expected aggregate error");
		let msg = err.to_string();
		assert!(msg.contains("boom-1"), "unexpected error: {msg}");
		assert!(msg.contains("boom-2"), "unexpected error: {msg}");
	}

	#[tokio::test]
	async fn fails_merge_when_strict_and_any_upstream_returns_terminal_error() {
		let failing = Messages(
			futures::stream::iter(vec![Ok(ServerJsonRpcMessage::error(
				rmcp::ErrorData::invalid_request("boom".to_string(), None),
				RequestId::Number(1),
			))])
			.boxed(),
		);
		let healthy = Messages(
			futures::stream::iter(vec![Ok(ServerJsonRpcMessage::response(
				ServerResult::empty(()),
				RequestId::Number(2),
			))])
			.boxed(),
		);

		let mut merge = MergeStream::new(
			vec![(Strng::from("bad"), failing), (Strng::from("ok"), healthy)],
			RequestId::Number(99),
			Box::new(|_| Ok(ServerResult::empty(()))),
			true,
			None,
		);

		let err = merge
			.next()
			.await
			.expect("message")
			.expect_err("should fail");
		assert!(
			err
				.to_string()
				.contains("one or more upstreams failed during merge")
		);
	}
}
