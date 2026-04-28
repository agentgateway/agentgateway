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
use tracing::{error, warn};

use crate::mcp::mergestream::Messages;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};

pub struct Process {
	sender: mpsc::Sender<(ClientJsonRpcMessage, IncomingRequestContext)>,
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

		if self
			.sender
			.send((JsonRpcMessage::Request(req), ctx.clone()))
			.await
			.is_err()
		{
			self.pending_requests.lock().unwrap().remove(&req_id);
			return Err(UpstreamError::Send);
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
			.sender
			.send((JsonRpcMessage::notification(req), ctx.clone()))
			.await
			.map_err(|_| UpstreamError::Send)?;
		Ok(())
	}
}

impl Process {
	pub fn new(mut proc: impl MCPTransport) -> Self {
		let (sender_tx, mut sender_rx) =
			mpsc::channel::<(ClientJsonRpcMessage, IncomingRequestContext)>(10);
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
						Some((msg, ctx)) => {
							if let Err(e) = proc.send(msg, &ctx).await {
								error!("Error sending message to stdio process: {:?}", e);
								terminal_err = Some(e);
								break;
							}
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
	) -> impl Future<Output = Result<(), UpstreamError>> + Send + 'static;

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
	) -> impl Future<Output = Result<(), UpstreamError>> + Send + 'static {
		Transport::send(self, item).map_err(Into::into)
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
		) -> impl Future<Output = Result<(), UpstreamError>> + Send + 'static {
			std::future::ready(Err(UpstreamError::InvalidRequest("boom".to_string())))
		}

		fn receive(&mut self) -> impl Future<Output = Option<ServerJsonRpcMessage>> + Send {
			std::future::pending()
		}

		fn close(&mut self) -> impl Future<Output = Result<(), UpstreamError>> + Send {
			std::future::ready(Ok(()))
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
