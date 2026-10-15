use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use agent_core::prelude::*;
use futures_util::TryFutureExt;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, JsonRpcMessage, JsonRpcRequest,
	RequestId, ServerJsonRpcMessage,
};
use rmcp::transport::{TokioChildProcess, Transport};
use tokio::sync::mpsc::Sender;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, warn};

use crate::mcp::mergestream::Messages;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};

pub struct Process {
	sender: mpsc::Sender<QueuedMessage>,
	shutdown_tx: agent_core::responsechannel::Sender<(), Option<UpstreamError>>,
	event_stream: AtomicOption<mpsc::Sender<ServerJsonRpcMessage>>,
	pending_requests: Arc<Mutex<HashMap<RequestId, oneshot::Sender<ServerJsonRpcMessage>>>>,
	alive: Arc<AtomicBool>,
}

impl Process {
	pub fn is_alive(&self) -> bool {
		self.alive.load(Ordering::Acquire)
	}

	pub async fn stop(&self) -> Result<(), UpstreamError> {
		if !self.is_alive() {
			return Ok(());
		}
		let res = self
			.shutdown_tx
			.send_and_wait(())
			.await
			.map_err(|_| UpstreamError::Send)?;
		if let Some(err) = res {
			Err(err)
		} else {
			Ok(())
		}
	}
	pub async fn send_message(
		&self,
		req: JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
	) -> Result<ServerJsonRpcMessage, UpstreamError> {
		if !self.is_alive() {
			return Err(UpstreamError::Recv);
		}
		let req_id = req.id.clone();
		let (sender, receiver) = oneshot::channel();

		self
			.pending_requests
			.lock()
			.unwrap()
			.insert(req_id.clone(), sender);

		if let Err(e) = self.queue(JsonRpcMessage::Request(req), ctx).await {
			self.pending_requests.lock().unwrap().remove(&req_id);
			return Err(match e {
				QueueError::Rejected(e) => e,
				QueueError::Stopped => UpstreamError::Recv,
			});
		}

		let response = receiver.await.map_err(|_| UpstreamError::Recv)?;
		Ok(response)
	}
	pub async fn get_event_stream(&self) -> Result<Messages, UpstreamError> {
		if !self.is_alive() {
			return Err(UpstreamError::Recv);
		}
		let (tx, rx) = tokio::sync::mpsc::channel(10);
		// This transport assumes a single active downstream event-stream consumer per
		// upstream session. Replacing the sender is acceptable for current MCP usage,
		// where one session owns one active GET/SSE stream, but it is not a general
		// multi-subscriber broadcast mechanism.
		self.event_stream.store(Some(Arc::new(tx)));
		Ok(Messages::from(rx))
	}
	pub async fn send_notification(
		&self,
		req: ClientNotification,
		ctx: &IncomingRequestContext,
	) -> Result<(), UpstreamError> {
		if !self.is_alive() {
			return Err(UpstreamError::Send);
		}
		self
			.queue(JsonRpcMessage::notification(req), ctx)
			.await
			.map_err(QueueError::into_send_error)
	}

	pub async fn send_client_message(
		&self,
		msg: ClientJsonRpcMessage,
		ctx: &IncomingRequestContext,
	) -> Result<(), UpstreamError> {
		if !self.is_alive() {
			return Err(UpstreamError::Send);
		}
		self
			.queue(msg, ctx)
			.await
			.map_err(QueueError::into_send_error)
	}

	/// Waits until the transport has sent the message.
	async fn queue(
		&self,
		msg: ClientJsonRpcMessage,
		ctx: &IncomingRequestContext,
	) -> Result<(), QueueError> {
		let (sent, sent_rx) = oneshot::channel();
		self
			.sender
			.send(QueuedMessage {
				msg,
				ctx: ctx.clone(),
				sent,
			})
			.await
			.map_err(|_| QueueError::Stopped)?;
		match sent_rx.await {
			Ok(Ok(())) => Ok(()),
			Ok(Err(e)) => Err(QueueError::Rejected(e)),
			// The transport stopped without sending this message.
			Err(_) => Err(QueueError::Stopped),
		}
	}
}

struct QueuedMessage {
	msg: ClientJsonRpcMessage,
	ctx: IncomingRequestContext,
	/// Completed once the message is sent or rejected. Dropped if the transport stops first.
	sent: oneshot::Sender<Result<(), UpstreamError>>,
}

enum QueueError {
	/// The transport rejected this message and remains usable.
	Rejected(UpstreamError),
	/// The transport stopped without sending this message.
	Stopped,
}

impl QueueError {
	fn into_send_error(self) -> UpstreamError {
		match self {
			QueueError::Rejected(e) => e,
			QueueError::Stopped => UpstreamError::Send,
		}
	}
}

/// A failed send, classified by whether the transport can keep sending.
#[derive(Debug)]
pub enum SendError {
	/// Only this message failed; the transport remains usable.
	Request(UpstreamError),
	/// The transport can no longer send messages.
	Terminal(UpstreamError),
}

impl Process {
	pub fn new(mut proc: impl MCPTransport) -> Self {
		let (sender_tx, mut sender_rx) = mpsc::channel::<QueuedMessage>(10);
		let (shutdown_tx, mut shutdown_rx) =
			agent_core::responsechannel::new::<(), Option<UpstreamError>>(10);
		let pending_requests = Arc::new(Mutex::new(HashMap::<
			RequestId,
			oneshot::Sender<ServerJsonRpcMessage>,
		>::new()));
		let pending_requests_clone = pending_requests.clone();
		let event_stream: AtomicOption<Sender<ServerJsonRpcMessage>> = Default::default();
		let event_stream_send: AtomicOption<Sender<ServerJsonRpcMessage>> = event_stream.clone();
		let alive = Arc::new(AtomicBool::new(true));
		let alive_task = alive.clone();

		tokio::spawn(async move {
			let mut terminal_err = None;
			let mut shutdown_resp = None;
			loop {
				tokio::select! {
					req = sender_rx.recv() => match req {
						Some(QueuedMessage { msg, ctx, sent }) => match proc.send(msg, &ctx).await {
							Ok(()) => {
								let _ = sent.send(Ok(()));
							},
							Err(SendError::Request(e)) => {
								let _ = sent.send(Err(e));
							},
							Err(SendError::Terminal(e)) => {
								error!("Error sending message to stdio process: {:?}", e);
								terminal_err = Some(e);
								break;
							},
						},
						None => break,
					},
					msg = proc.receive() => {
						match msg {
							Some(JsonRpcMessage::Response(res)) => {
								let req_id = res.id.clone();
								if let Some(sender) = pending_requests_clone.lock().unwrap().remove(&req_id) {
									let _ = sender.send(ServerJsonRpcMessage::Response(res));
								}
							},
							Some(JsonRpcMessage::Error(err)) => {
								// An error carrying a request id terminates that request, like a response.
								// An id-less error can't be matched, so it belongs on the event stream.
								match err.id.as_ref() {
									Some(id) => {
										if let Some(sender) = pending_requests_clone.lock().unwrap().remove(id) {
											let _ = sender.send(ServerJsonRpcMessage::Error(err));
										} else {
											debug!("dropping stdio error for unknown request id {id:?}");
										}
									},
									None => {
										if let Some(sender) = event_stream_send.load().as_ref() {
											let _ = sender.send(JsonRpcMessage::Error(err)).await;
										}
									},
								}
							},
							Some(other) => {
								if let Some(sender) = event_stream_send.load().as_ref() {
									let _ = sender.send(other).await;
								}
							},
							None => {
								terminal_err = Some(UpstreamError::StdioShutdown);
								break;
							}
						}
					},
					req = shutdown_rx.recv() => match req {
						Some((_, resp)) => {
							shutdown_resp = Some(resp);
							break;
						},
						None => break,
					},
				}
			}

			alive_task.store(false, Ordering::Release);
			event_stream_send.store(None);
			pending_requests_clone.lock().unwrap().clear();

			let close_err = proc.close().await.err();
			if let Some(e) = close_err.as_ref() {
				warn!("Error shutting down stdio process: {:?}", e);
			}
			if let Some(resp) = shutdown_resp {
				let _ = resp.send(terminal_err.or(close_err));
			}
		});

		Self {
			sender: sender_tx,
			shutdown_tx,
			event_stream,
			pending_requests,
			alive,
		}
	}
}

impl Debug for Process {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Process").finish()
	}
}

pub trait MCPTransport: Send + 'static {
	/// Send a message to the transport
	///
	/// Notice that the future returned by this function should be `Send` and `'static`.
	/// It's because the sending message could be executed concurrently.
	fn send(
		&mut self,
		item: ClientJsonRpcMessage,
		user_headers: &IncomingRequestContext,
	) -> impl Future<Output = Result<(), SendError>> + Send + 'static;

	/// Receive a message from the transport, this operation is sequential.
	fn receive(&mut self) -> impl Future<Output = Option<ServerJsonRpcMessage>> + Send;

	/// Close the transport
	fn close(&mut self) -> impl Future<Output = Result<(), UpstreamError>> + Send;
}

impl MCPTransport for TokioChildProcess {
	fn send(
		&mut self,
		item: ClientJsonRpcMessage,
		_: &IncomingRequestContext,
	) -> impl Future<Output = Result<(), SendError>> + Send + 'static {
		Transport::send(self, item).map_err(|e| SendError::Terminal(e.into()))
	}

	fn receive(&mut self) -> impl Future<Output = Option<ServerJsonRpcMessage>> + Send {
		Transport::receive(self)
	}

	fn close(&mut self) -> impl Future<Output = Result<(), UpstreamError>> + Send {
		Transport::close(self).map_err(Into::into)
	}
}

#[cfg(test)]
mod tests {
	use futures_util::StreamExt;
	use rmcp::model::{ClientRequest, JsonRpcRequest, RequestId};
	use tokio::time::{Duration, timeout};

	use super::*;

	struct FailOnSendTransport;

	impl MCPTransport for FailOnSendTransport {
		fn send(
			&mut self,
			_: ClientJsonRpcMessage,
			_: &IncomingRequestContext,
		) -> impl Future<Output = Result<(), SendError>> + Send + 'static {
			std::future::ready(Err(SendError::Terminal(UpstreamError::InvalidRequest(
				"boom".to_string(),
			))))
		}

		fn receive(&mut self) -> impl Future<Output = Option<ServerJsonRpcMessage>> + Send {
			std::future::pending()
		}

		fn close(&mut self) -> impl Future<Output = Result<(), UpstreamError>> + Send {
			std::future::ready(Ok(()))
		}
	}

	struct ErrorReplyTransport {
		tx: mpsc::Sender<ServerJsonRpcMessage>,
		rx: mpsc::Receiver<ServerJsonRpcMessage>,
	}

	impl MCPTransport for ErrorReplyTransport {
		fn send(
			&mut self,
			item: ClientJsonRpcMessage,
			_: &IncomingRequestContext,
		) -> impl Future<Output = Result<(), SendError>> + Send + 'static {
			let tx = self.tx.clone();
			async move {
				if let JsonRpcMessage::Request(req) = item {
					let err = rmcp::model::JsonRpcError::new(
						Some(req.id),
						rmcp::ErrorData::invalid_request(
							"connection is locked to the legacy handshake era",
							None,
						),
					);
					let _ = tx.send(ServerJsonRpcMessage::Error(err)).await;
				}
				Ok(())
			}
		}

		fn receive(&mut self) -> impl Future<Output = Option<ServerJsonRpcMessage>> + Send {
			self.rx.recv()
		}

		fn close(&mut self) -> impl Future<Output = Result<(), UpstreamError>> + Send {
			std::future::ready(Ok(()))
		}
	}

	/// Rejects request 2 and every non-request message; the test supplies replies for other
	/// requests.
	struct RejectingTransport {
		rx: mpsc::Receiver<ServerJsonRpcMessage>,
	}

	impl MCPTransport for RejectingTransport {
		fn send(
			&mut self,
			item: ClientJsonRpcMessage,
			_: &IncomingRequestContext,
		) -> impl Future<Output = Result<(), SendError>> + Send + 'static {
			let rejected = match &item {
				JsonRpcMessage::Request(req) => req.id == RequestId::Number(2),
				_ => true,
			};
			std::future::ready(if rejected {
				Err(SendError::Request(UpstreamError::InvalidRequest(
					"rejected".to_string(),
				)))
			} else {
				Ok(())
			})
		}

		fn receive(&mut self) -> impl Future<Output = Option<ServerJsonRpcMessage>> + Send {
			self.rx.recv()
		}

		fn close(&mut self) -> impl Future<Output = Result<(), UpstreamError>> + Send {
			std::future::ready(Ok(()))
		}
	}

	fn ping(id: i64) -> JsonRpcRequest<ClientRequest> {
		JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(id),
			request: ClientRequest::PingRequest(Default::default()),
		}
	}

	async fn wait_until_pending(proc: &Process, id: i64) {
		timeout(Duration::from_secs(1), async {
			while !proc
				.pending_requests
				.lock()
				.unwrap()
				.contains_key(&RequestId::Number(id))
			{
				tokio::task::yield_now().await;
			}
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn test_process_request_send_failure_keeps_transport_usable() {
		let (tx, rx) = mpsc::channel(4);
		let proc = Arc::new(Process::new(RejectingTransport { rx }));
		let ctx = IncomingRequestContext::empty();
		let mut events = proc.get_event_stream().await.unwrap();
		let first = tokio::spawn({
			let proc = proc.clone();
			async move {
				proc
					.send_message(ping(1), &IncomingRequestContext::empty())
					.await
			}
		});
		wait_until_pending(&proc, 1).await;

		let err = proc.send_message(ping(2), &ctx).await.unwrap_err();
		assert!(matches!(err, UpstreamError::InvalidRequest(ref msg) if msg == "rejected"));
		let err = proc
			.send_notification(
				ClientNotification::InitializedNotification(Default::default()),
				&ctx,
			)
			.await
			.unwrap_err();
		assert!(matches!(err, UpstreamError::InvalidRequest(ref msg) if msg == "rejected"));
		let err = proc
			.send_client_message(
				ClientJsonRpcMessage::response(rmcp::model::ClientResult::empty(()), RequestId::Number(9)),
				&ctx,
			)
			.await
			.unwrap_err();
		assert!(matches!(err, UpstreamError::InvalidRequest(ref msg) if msg == "rejected"));
		assert!(proc.is_alive());
		{
			let pending = proc.pending_requests.lock().unwrap();
			assert!(!pending.contains_key(&RequestId::Number(2)));
			assert!(pending.contains_key(&RequestId::Number(1)));
		}

		let third = tokio::spawn({
			let proc = proc.clone();
			async move {
				proc
					.send_message(ping(3), &IncomingRequestContext::empty())
					.await
			}
		});
		wait_until_pending(&proc, 3).await;
		for id in [3, 1] {
			tx.send(ServerJsonRpcMessage::response(
				rmcp::model::ServerResult::empty(()),
				RequestId::Number(id),
			))
			.await
			.unwrap();
		}
		tx.send(ServerJsonRpcMessage::notification(
			rmcp::model::ServerNotification::ToolListChangedNotification(Default::default()),
		))
		.await
		.unwrap();

		for (task, id) in [(third, 3), (first, 1)] {
			let resp = timeout(Duration::from_secs(1), task)
				.await
				.unwrap()
				.unwrap()
				.unwrap();
			match resp {
				ServerJsonRpcMessage::Response(r) => assert_eq!(r.id, RequestId::Number(id)),
				other => panic!("expected response, got {other:?}"),
			}
		}
		let event = timeout(Duration::from_secs(1), events.next())
			.await
			.unwrap();
		assert!(matches!(
			event,
			Some(Ok(ServerJsonRpcMessage::Notification(_)))
		));
	}

	#[tokio::test]
	async fn test_process_error_reply_resolves_pending_request() {
		let (tx, rx) = mpsc::channel(4);
		let proc = Process::new(ErrorReplyTransport { tx, rx });
		let req = JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(1),
			request: ClientRequest::PingRequest(Default::default()),
		};

		let resp = timeout(
			Duration::from_secs(1),
			proc.send_message(req, &IncomingRequestContext::empty()),
		)
		.await
		.expect("an error reply must resolve the pending request, not hang")
		.unwrap();

		match resp {
			ServerJsonRpcMessage::Error(e) => assert_eq!(e.id, Some(RequestId::Number(1))),
			other => panic!("expected error reply, got {other:?}"),
		}
	}

	#[tokio::test]
	async fn test_process_fails_pending_requests_when_transport_dies() {
		let proc = Process::new(FailOnSendTransport);
		let req = JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(1),
			request: ClientRequest::PingRequest(Default::default()),
		};

		let err = proc
			.send_message(req, &IncomingRequestContext::empty())
			.await
			.unwrap_err();

		assert!(matches!(err, UpstreamError::Recv));
		assert!(!proc.is_alive());
	}

	#[tokio::test]
	async fn test_process_fails_notification_when_transport_dies() {
		let proc = Process::new(FailOnSendTransport);

		let err = proc
			.send_notification(
				ClientNotification::InitializedNotification(Default::default()),
				&IncomingRequestContext::empty(),
			)
			.await
			.unwrap_err();

		assert!(matches!(err, UpstreamError::Send));
		assert!(!proc.is_alive());
	}

	/// Holds the first send until released, then fails every send as terminal.
	struct HeldFailureTransport {
		started: Option<oneshot::Sender<()>>,
		release: Option<oneshot::Receiver<()>>,
	}

	impl MCPTransport for HeldFailureTransport {
		fn send(
			&mut self,
			_: ClientJsonRpcMessage,
			_: &IncomingRequestContext,
		) -> impl Future<Output = Result<(), SendError>> + Send + 'static {
			if let Some(started) = self.started.take() {
				let _ = started.send(());
			}
			let release = self.release.take();
			async move {
				if let Some(release) = release {
					let _ = release.await;
				}
				Err(SendError::Terminal(UpstreamError::InvalidRequest(
					"boom".to_string(),
				)))
			}
		}

		fn receive(&mut self) -> impl Future<Output = Option<ServerJsonRpcMessage>> + Send {
			std::future::pending()
		}

		fn close(&mut self) -> impl Future<Output = Result<(), UpstreamError>> + Send {
			std::future::ready(Ok(()))
		}
	}

	#[tokio::test]
	async fn test_process_fails_queued_notification_when_transport_dies() {
		let (started_tx, started_rx) = oneshot::channel();
		let (release_tx, release_rx) = oneshot::channel();
		let proc = Arc::new(Process::new(HeldFailureTransport {
			started: Some(started_tx),
			release: Some(release_rx),
		}));
		let notify = |proc: Arc<Process>| async move {
			proc
				.send_notification(
					ClientNotification::InitializedNotification(Default::default()),
					&IncomingRequestContext::empty(),
				)
				.await
		};

		let in_flight = tokio::spawn(notify(proc.clone()));
		timeout(Duration::from_secs(1), started_rx)
			.await
			.unwrap()
			.unwrap();
		let queued = tokio::spawn(notify(proc.clone()));
		timeout(Duration::from_secs(1), async {
			while proc.sender.capacity() == proc.sender.max_capacity() {
				tokio::task::yield_now().await;
			}
		})
		.await
		.unwrap();
		release_tx.send(()).unwrap();

		for task in [in_flight, queued] {
			let err = timeout(Duration::from_secs(1), task)
				.await
				.unwrap()
				.unwrap()
				.unwrap_err();
			assert!(matches!(err, UpstreamError::Send));
		}
		assert!(!proc.is_alive());
	}

	#[tokio::test]
	async fn test_process_closes_event_stream_when_transport_dies() {
		let proc = Process::new(FailOnSendTransport);
		let mut events = proc.get_event_stream().await.unwrap();
		let req = JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(1),
			request: ClientRequest::PingRequest(Default::default()),
		};

		let _ = proc
			.send_message(req, &IncomingRequestContext::empty())
			.await;

		let next = timeout(Duration::from_secs(1), events.next())
			.await
			.unwrap();
		assert!(next.is_none());
	}

	#[tokio::test]
	async fn test_process_rejects_new_event_stream_when_dead() {
		let proc = Process::new(FailOnSendTransport);
		let req = JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(1),
			request: ClientRequest::PingRequest(Default::default()),
		};

		let _ = proc
			.send_message(req, &IncomingRequestContext::empty())
			.await;

		let err = match proc.get_event_stream().await {
			Ok(_) => panic!("expected dead process to reject new event stream"),
			Err(err) => err,
		};
		assert!(matches!(err, UpstreamError::Recv));
	}
}
