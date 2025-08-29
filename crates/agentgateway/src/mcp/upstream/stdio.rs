use crate::mcp::mergestream::Messages;
use crate::mcp::upstream::UpstreamError;
use agent_core::prelude::*;
use futures::StreamExt;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, JsonRpcMessage, JsonRpcRequest,
	RequestId, ServerJsonRpcMessage,
};
use rmcp::transport::{TokioChildProcess, Transport};
use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;
use tokio::sync::{mpsc, oneshot};
use tracing::{error, warn};

pub struct Process {
	sender: mpsc::Sender<ClientJsonRpcMessage>,
	shutdown_tx: agent_core::responsechannel::Sender<(), Option<UpstreamError>>,
	event_stream: AtomicOption<mpsc::Sender<ServerJsonRpcMessage>>,
	pending_requests: Arc<Mutex<HashMap<RequestId, oneshot::Sender<ServerJsonRpcMessage>>>>,
}

impl Process {
	pub async fn stop(&self) -> Result<(), UpstreamError> {
		let res = self
			.shutdown_tx
			.send(())
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
	) -> Result<oneshot::Receiver<ServerJsonRpcMessage>, UpstreamError> {
		let req_id = req.id.clone();
		let (sender, receiver) = oneshot::channel();

		self.pending_requests.lock().unwrap().insert(req_id, sender);

		self
			.sender
			.send(JsonRpcMessage::Request(req))
			.await
			.map_err(|_| UpstreamError::Send)?;

		Ok(receiver)
	}
	pub async fn get_event_stream(&self) -> Messages {
		let (tx, rx) = tokio::sync::mpsc::channel(10);
		self.event_stream.store(Some(Arc::new(tx)));
		Messages::from(rx)
	}
	pub async fn send_notification(&self, req: ClientNotification) -> Result<(), UpstreamError> {
		self
			.sender
			.send(JsonRpcMessage::notification(req))
			.await
			.map_err(|_| UpstreamError::Send)?;
		Ok(())
	}
}

impl Process {
	pub fn new(mut proc: TokioChildProcess) -> Self {
		let (sender_tx, mut sender_rx) = mpsc::channel::<ClientJsonRpcMessage>(10);
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
					Some(msg) = sender_rx.recv() => {
						if let Err(e) = proc.send(msg).await {
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
						let err = proc.graceful_shutdown().await;
						if let Err(e) = &err {
							warn!("Error shutting down stdio process: {:?}", e);
						}
						let _ = resp.send(err.err().map(Into::into));
						return;
					},
					else => {
						let err = proc.graceful_shutdown().await;
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

	// pub async fn notify(&self, notif: ClientNotification) -> Result<(), UpstreamError> {
	// 	self
	// 		.sender
	// 		.send(JsonRpcMessage::Notification(notif.into()))
	// 		.await
	// 		.map_err(|_| UpstreamError::InvalidRequest("TODO".to_string()))?;
	// 	Ok(())
	// }
}

impl Debug for Process {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Process").finish()
	}
}
