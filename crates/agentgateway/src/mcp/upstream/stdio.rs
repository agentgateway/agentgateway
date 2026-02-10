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
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{error, warn};

use crate::mcp::mergestream::Messages;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};

pub struct Process {
	sender: mpsc::Sender<(ClientJsonRpcMessage, IncomingRequestContext)>,
	shutdown_tx: agent_core::responsechannel::Sender<(), Option<UpstreamError>>,
	event_stream: AtomicOption<mpsc::Sender<ServerJsonRpcMessage>>,
	event_bus: broadcast::Sender<ServerJsonRpcMessage>,
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
	pub async fn send_message_stream(
		&self,
		req: JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
	) -> Result<Messages, UpstreamError> {
		let req_id = req.id.clone();
		let (response_tx, mut response_rx) = oneshot::channel();
		self
			.pending_requests
			.lock()
			.unwrap()
			.insert(req_id.clone(), response_tx);

		let mut event_rx = self.event_bus.subscribe();
		if self
			.sender
			.send((JsonRpcMessage::Request(req), ctx.clone()))
			.await
			.is_err()
		{
			self.pending_requests.lock().unwrap().remove(&req_id);
			return Err(UpstreamError::Send);
		}

		let pending_requests = self.pending_requests.clone();
		let (tx, rx) = mpsc::channel(16);
		tokio::spawn(async move {
			loop {
				tokio::select! {
					response = &mut response_rx => {
						match response {
							Ok(msg) => {
								let _ = tx.send(msg).await;
							},
							Err(_) => {
								pending_requests.lock().unwrap().remove(&req_id);
								let _ = tx
									.send(ServerJsonRpcMessage::error(
										rmcp::ErrorData::internal_error(
											"upstream closed on receive".to_string(),
											None,
										),
										req_id.clone(),
									))
									.await;
							},
						}
						break;
					},
					msg = event_rx.recv() => {
						match msg {
							Ok(msg) => {
								if tx.send(msg).await.is_err() {
									break;
								}
							},
							Err(broadcast::error::RecvError::Lagged(_)) => {},
							Err(broadcast::error::RecvError::Closed) => break,
						}
					},
				}
			}
		});

		Ok(Messages::from(rx))
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

	pub async fn send_raw(
		&self,
		msg: ClientJsonRpcMessage,
		ctx: &IncomingRequestContext,
	) -> Result<(), UpstreamError> {
		self
			.sender
			.send((msg, ctx.clone()))
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
		let (event_bus, _) = broadcast::channel::<ServerJsonRpcMessage>(32);
		let pending_requests = Arc::new(Mutex::new(HashMap::<
			RequestId,
			oneshot::Sender<ServerJsonRpcMessage>,
		>::new()));
		let pending_requests_clone = pending_requests.clone();
		let event_bus_send = event_bus.clone();
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
									match sender.send(other).await {
										Ok(()) => {},
										Err(err) => {
											let _ = event_bus_send.send(err.0);
										},
									}
								} else {
									let _ = event_bus_send.send(other);
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
			event_bus,
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
