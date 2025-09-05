use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Formatter};
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
}

impl Process {
	pub async fn stop(&self) -> Result<(), UpstreamError> {
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
		let req_id = req.id.clone();
		let (sender, receiver) = oneshot::channel();

		self.pending_requests.lock().unwrap().insert(req_id, sender);

		self
			.sender
			.send((JsonRpcMessage::Request(req), ctx.clone()))
			.await
			.map_err(|_| UpstreamError::Send)?;

		let response = receiver.await.map_err(|_| UpstreamError::Recv)?;
		Ok(response)
	}
	pub async fn get_event_stream(&self) -> Messages {
		let (tx, rx) = tokio::sync::mpsc::channel(10);
		self.event_stream.store(Some(Arc::new(tx)));
		Messages::from(rx)
	}
	pub async fn send_notification(
		&self,
		req: ClientNotification,
		ctx: &IncomingRequestContext,
	) -> Result<(), UpstreamError> {
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

		tokio::spawn(async move {
			loop {
				tokio::select! {
					Some((msg, ctx)) = sender_rx.recv() => {
						if let Err(e) = proc.send(msg, &ctx).await {
							error!("Error sending message to stdio process: {:?}", e);
							break;
						}
					},
					Some(msg) = proc.receive() => {
						match msg {
							JsonRpcMessage::Response(res) => {
								let req_id = res.id.clone();
								if let Some(sender) = pending_requests_clone.lock().unwrap().remove(&req_id) {
									let _ = sender.send(ServerJsonRpcMessage::Response(res));
								}
							},
							other => {
								if let Some(sender) = event_stream_send.load().as_ref() {
									let _ = sender.send(other).await;
								}
							}
						}
					},
					Some((_, resp)) = shutdown_rx.recv() => {
						let err = proc.close().await;
						if let Err(e) = &err {
							warn!("Error shutting down stdio process: {:?}", e);
						}
						let _ = resp.send(err.err());
						return;
					},
					else => {
						let err = proc.close().await;
						if let Err(e) = err {
							warn!("Error shutting down stdio process: {:?}", e);
						}
						return;
					},
				}
			}
		});

		Self {
			sender: sender_tx,
			shutdown_tx,
			event_stream,
			pending_requests,
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
