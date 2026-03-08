use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::Arc;

use ::http::StatusCode;
use ::http::header::CONTENT_TYPE;
use ::http::request::Parts;
use agent_core::version::BuildInfo;
use anyhow::anyhow;
use futures_util::StreamExt;
use headers::HeaderMapExt;
use rmcp::ErrorData;
use rmcp::model::{
	ClientInfo, ClientJsonRpcMessage, ClientNotification, ClientRequest, ConstString, ErrorCode,
	Implementation, JsonRpcError, JsonRpcRequest, ProtocolVersion, RequestId, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{
	EVENT_STREAM_MIME_TYPE, HEADER_MCP_PROTOCOL_VERSION, JSON_MIME_TYPE,
};
use serde::{Deserialize, Serialize};
use sse_stream::{KeepAlive, Sse, SseBody, SseStream};
use tokio::sync::{
	Mutex, Notify, broadcast,
	mpsc::{Receiver, Sender},
};

use crate::http::{Response, Uri};
use crate::mcp::mergestream::{MergeFn, Messages};
use crate::mcp::relay::{Relay, RelayInputs};
use crate::mcp::streamablehttp::{ServerSseMessage, StreamableHttpPostResponse};
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{
	ClientError, MCPOperation, ResumeFailureReason, local_session_binding, rbac, session_binding_tag,
};
use crate::proxy::ProxyError;
use crate::{mcp, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionContinuity {
	Reconstructible,
	LiveOnly,
	OneShot,
}

impl SessionContinuity {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Reconstructible => "reconstructible",
			Self::LiveOnly => "live_only",
			Self::OneShot => "one_shot",
		}
	}
}

impl std::fmt::Display for SessionContinuity {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.as_str())
	}
}

const LAST_EVENT_ID_HEADER: &str = "last-event-id";
const STREAM_EVENT_ID_KIND: &str = "agw-stream";
const STREAM_EVENT_ID_VERSION: u8 = 1;
const STREAM_REPLAY_BUFFER_CAPACITY: usize = 256;
const STREAM_REPLAY_CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StreamEventId {
	#[serde(rename = "t")]
	kind: String,
	#[serde(rename = "v")]
	version: u8,
	#[serde(rename = "b")]
	session_binding: String,
	#[serde(rename = "s")]
	sequence: u64,
}

impl StreamEventId {
	fn new(session_binding: &str, sequence: u64) -> Self {
		Self {
			kind: STREAM_EVENT_ID_KIND.to_string(),
			version: STREAM_EVENT_ID_VERSION,
			session_binding: session_binding.to_string(),
			sequence,
		}
	}
}

#[derive(Debug, Clone)]
struct ReplayRecord {
	sequence: u64,
	message: ServerSseMessage,
}

enum ReplayPumpState {
	Stopped,
	Starting(Arc<Notify>),
	Running {
		sender: broadcast::Sender<ServerSseMessage>,
		task: Option<tokio::task::JoinHandle<()>>,
	},
}

struct SessionReplayInner {
	next_sequence: u64,
	buffer: VecDeque<ReplayRecord>,
	pump: ReplayPumpState,
}

struct SessionReplaySubscription {
	replay: Vec<ServerSseMessage>,
	receiver: broadcast::Receiver<ServerSseMessage>,
}

#[derive(Clone)]
struct SessionReplayState {
	encoder: http::sessionpersistence::Encoder,
	session_binding: Arc<std::sync::RwLock<String>>,
	inner: Arc<Mutex<SessionReplayInner>>,
}

impl std::fmt::Debug for SessionReplayState {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SessionReplayState").finish_non_exhaustive()
	}
}

impl SessionReplayState {
	fn new(encoder: http::sessionpersistence::Encoder) -> Self {
		Self {
			encoder,
			session_binding: Arc::new(std::sync::RwLock::new(local_session_binding())),
			inner: Arc::new(Mutex::new(SessionReplayInner {
				next_sequence: 0,
				buffer: VecDeque::with_capacity(STREAM_REPLAY_BUFFER_CAPACITY),
				pump: ReplayPumpState::Stopped,
			})),
		}
	}

	fn set_session_binding(&self, session_handle: &str) {
		let mut binding = self.session_binding.write().unwrap_or_else(|e| {
			tracing::error!("session replay binding lock poisoned while updating; continuing");
			e.into_inner()
		});
		*binding = session_binding_tag(session_handle);
	}

	async fn open_stream(
		&self,
		relay: Arc<Relay>,
		ctx: IncomingRequestContext,
		last_event_id: Option<&str>,
	) -> Result<SessionReplaySubscription, UpstreamError> {
		loop {
			enum OpenAction {
				Wait(Arc<Notify>),
				Start(Arc<Notify>),
				Ready(SessionReplaySubscription),
			}

			let action = {
				let mut inner = self.inner.lock().await;
				match &inner.pump {
					ReplayPumpState::Running { sender, .. } => {
						let replay = self.replay_from(&inner, last_event_id);
						let receiver = sender.subscribe();
						OpenAction::Ready(SessionReplaySubscription { replay, receiver })
					},
					ReplayPumpState::Starting(notify) => OpenAction::Wait(notify.clone()),
					ReplayPumpState::Stopped => {
						let notify = Arc::new(Notify::new());
						inner.pump = ReplayPumpState::Starting(notify.clone());
						OpenAction::Start(notify)
					},
				}
			};

			match action {
				OpenAction::Ready(subscription) => return Ok(subscription),
				OpenAction::Wait(notify) => {
					notify.notified().await;
				},
				OpenAction::Start(notify) => {
					let messages = relay.get_event_stream_messages(ctx.clone()).await;
					match messages {
						Ok(messages) => {
							let subscription = {
								let mut inner = self.inner.lock().await;
								let ReplayPumpState::Starting(current_notify) = &inner.pump else {
									continue;
								};
								if !Arc::ptr_eq(current_notify, &notify) {
									continue;
								}
								let (sender, _) = broadcast::channel(STREAM_REPLAY_CHANNEL_CAPACITY);
								let receiver = sender.subscribe();
								let replay = self.replay_from(&inner, last_event_id);
								inner.pump = ReplayPumpState::Running { sender, task: None };
								SessionReplaySubscription { replay, receiver }
							};
							notify.notify_waiters();
							self.attach_stream_task(messages).await;
							return Ok(subscription);
						},
						Err(error) => {
							let wake = {
								let mut inner = self.inner.lock().await;
								let Some(current_notify) = (match &inner.pump {
									ReplayPumpState::Starting(current_notify)
										if Arc::ptr_eq(current_notify, &notify) =>
									{
										Some(current_notify.clone())
									},
									_ => None,
								}) else {
									continue;
								};
								inner.pump = ReplayPumpState::Stopped;
								current_notify
							};
							wake.notify_waiters();
							return Err(error);
						},
					}
				},
			}
		}
	}

	async fn attach_stream_task(&self, messages: Messages) {
		let replay = self.clone();
		let task = tokio::spawn(async move {
			replay.run_stream(messages).await;
		});
		let mut inner = self.inner.lock().await;
		if let ReplayPumpState::Running { task: slot, .. } = &mut inner.pump {
			*slot = Some(task);
			return;
		}
		drop(inner);
		task.abort();
	}

	#[cfg(test)]
	async fn subscribe(&self, last_event_id: Option<&str>) -> Option<SessionReplaySubscription> {
		let inner = self.inner.lock().await;
		let ReplayPumpState::Running { sender, .. } = &inner.pump else {
			return None;
		};
		let replay = self.replay_from(&inner, last_event_id);
		let receiver = sender.subscribe();
		Some(SessionReplaySubscription { replay, receiver })
	}

	fn replay_from(
		&self,
		inner: &SessionReplayInner,
		last_event_id: Option<&str>,
	) -> Vec<ServerSseMessage> {
		let Some(last_event_id) = last_event_id else {
			return Vec::new();
		};
		// Replay is intentionally best-effort and same-instance only. Any cursor
		// we cannot validate against the current live session falls back to a
		// fresh stream instead of pretending cross-instance continuity exists.
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("session replay binding lock poisoned while reading; continuing");
			e.into_inner()
		});
		let stream_event_id = match self.decode_event_id(last_event_id) {
			Ok(id) => id,
			Err(reason) => {
				tracing::warn!(
					%reason,
					last_event_id,
					"ignoring Last-Event-ID for MCP replay"
				);
				return Vec::new();
			},
		};
		if stream_event_id.session_binding != session_binding.as_str() {
			tracing::warn!(
				last_event_id,
				"ignoring Last-Event-ID from a different MCP session binding"
			);
			return Vec::new();
		}
		if stream_event_id.sequence >= inner.next_sequence {
			tracing::warn!(
				last_event_id,
				"ignoring Last-Event-ID that is ahead of the current MCP replay cursor"
			);
			return Vec::new();
		}
		let Some(oldest_sequence) = inner.buffer.front().map(|record| record.sequence) else {
			return Vec::new();
		};
		if stream_event_id.sequence < oldest_sequence {
			tracing::warn!(
				last_event_id,
				oldest_sequence,
				"Last-Event-ID fell out of the local MCP replay buffer; starting a fresh stream"
			);
			return Vec::new();
		}
		inner
			.buffer
			.iter()
			.filter(|record| record.sequence > stream_event_id.sequence)
			.map(|record| record.message.clone())
			.collect()
	}

	fn encode_event_id(&self, sequence: u64) -> Result<String, http::sessionpersistence::Error> {
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("session replay binding lock poisoned while encoding; continuing");
			e.into_inner()
		});
		let event_id = StreamEventId::new(session_binding.as_str(), sequence);
		self
			.encoder
			.encrypt(&serde_json::to_string(&event_id).expect("stream event id should serialize"))
	}

	fn decode_event_id(&self, event_id: &str) -> Result<StreamEventId, &'static str> {
		let decoded = self
			.encoder
			.decrypt(event_id)
			.map_err(|_| "stream event id decode failed")?;
		let event_id = serde_json::from_slice::<StreamEventId>(&decoded)
			.map_err(|_| "stream event id payload invalid")?;
		if event_id.kind != STREAM_EVENT_ID_KIND {
			return Err("stream event id kind invalid");
		}
		if event_id.version != STREAM_EVENT_ID_VERSION {
			return Err("stream event id version invalid");
		}
		Ok(event_id)
	}

	async fn publish_message(&self, message: ServerJsonRpcMessage) {
		let mut inner = self.inner.lock().await;
		let ReplayPumpState::Running { sender, .. } = &inner.pump else {
			return;
		};
		let sender = sender.clone();
		let sequence = inner.next_sequence;
		inner.next_sequence += 1;
		let event_id = match self.encode_event_id(sequence) {
			Ok(id) => Some(id),
			Err(error) => {
				tracing::warn!(%error, sequence, "failed to encode MCP stream event id");
				None
			},
		};
		let message = ServerSseMessage {
			event_id,
			message: Arc::new(message),
		};
		inner.buffer.push_back(ReplayRecord {
			sequence,
			message: message.clone(),
		});
		while inner.buffer.len() > STREAM_REPLAY_BUFFER_CAPACITY {
			inner.buffer.pop_front();
		}
		let _ = sender.send(message);
	}

	async fn run_stream(&self, mut messages: Messages) {
		while let Some(message) = messages.next().await {
			let message = match message {
				Ok(message) => message,
				Err(error) => ServerJsonRpcMessage::error(
					ErrorData::internal_error(error.to_string(), None),
					RequestId::Number(0),
				),
			};
			self.publish_message(message).await;
		}
		self.stop().await;
	}

	async fn stop(&self) {
		let mut inner = self.inner.lock().await;
		inner.buffer.clear();
		inner.pump = ReplayPumpState::Stopped;
	}

	async fn shutdown(&self) {
		let task = {
			let mut inner = self.inner.lock().await;
			inner.buffer.clear();
			match std::mem::replace(&mut inner.pump, ReplayPumpState::Stopped) {
				ReplayPumpState::Running { task, .. } => task,
				_ => None,
			}
		};
		if let Some(task) = task {
			task.abort();
		}
	}

	#[cfg(test)]
	async fn start_test_stream(&self, messages: Messages) -> SessionReplaySubscription {
		let subscription = {
			let mut inner = self.inner.lock().await;
			let (sender, _) = broadcast::channel(STREAM_REPLAY_CHANNEL_CAPACITY);
			let receiver = sender.subscribe();
			inner.pump = ReplayPumpState::Running { sender, task: None };
			SessionReplaySubscription {
				replay: Vec::new(),
				receiver,
			}
		};
		self.attach_stream_task(messages).await;
		subscription
	}
}

#[derive(Debug, Clone)]
pub struct Session {
	encoder: http::sessionpersistence::Encoder,
	relay: Arc<Relay>,
	replay: SessionReplayState,
	pub id: Arc<str>,
	continuity: SessionContinuity,
	tx: Option<Sender<ServerJsonRpcMessage>>,
}

impl Session {
	pub const fn continuity(&self) -> SessionContinuity {
		self.continuity
	}

	fn downgrade_continuity(&mut self, continuity: SessionContinuity) {
		if self.continuity == continuity {
			return;
		}
		tracing::debug!(
			from = %self.continuity,
			to = %continuity,
			"updated MCP session continuity"
		);
		self.continuity = continuity;
	}

	/// send a message to upstream server(s)
	pub async fn send(
		&mut self,
		parts: Parts,
		message: ClientJsonRpcMessage,
	) -> Result<Response, ProxyError> {
		let req_id = match &message {
			ClientJsonRpcMessage::Request(r) => Some(r.id.clone()),
			_ => None,
		};
		let req_uri = parts.uri.clone();
		Self::handle_error(
			req_id,
			Some(req_uri),
			self.send_internal(parts, message).await,
		)
		.await
	}
	/// send a message to upstream server(s), when using stateless mode. In stateless mode, every message
	/// is wrapped in an InitializeRequest (except the actual InitializeRequest from the downstream).
	/// This ensures servers that require an InitializeRequest behave correctly.
	/// In the future, we may have a mode where we know the downstream is stateless as well, and can just forward as-is.
	pub async fn stateless_send_and_initialize(
		&mut self,
		parts: Parts,
		message: ClientJsonRpcMessage,
	) -> Result<Response, ProxyError> {
		let request_uri = parts.uri.clone();
		let req_id = match &message {
			ClientJsonRpcMessage::Request(r) => Some(r.id.clone()),
			_ => None,
		};
		let is_init = matches!(&message, ClientJsonRpcMessage::Request(r) if matches!(&r.request, &ClientRequest::InitializeRequest(_)));
		if !is_init {
			let synthetic_init_protocol_version = match protocol_version_from_headers(&parts.headers) {
				Ok(version) => version,
				Err(err) => {
					return Self::handle_error(req_id, Some(request_uri), Err(err)).await;
				},
			};
			// first, send the initialize
			let init_request =
				rmcp::model::InitializeRequest::new(get_client_info(synthetic_init_protocol_version));
			let _ = self
				.send(
					parts.clone(),
					ClientJsonRpcMessage::request(init_request.into(), RequestId::Number(0)),
				)
				.await?;

			// And we need to notify as well.
			let notification = ClientJsonRpcMessage::notification(
				rmcp::model::InitializedNotification {
					method: Default::default(),
					extensions: Default::default(),
				}
				.into(),
			);
			let _ = self.send(parts.clone(), notification).await?;
		}
		// Now we can send the message like normal
		self.send(parts, message).await
	}

	pub fn with_inputs(mut self, inputs: RelayInputs) -> Self {
		self.relay = Arc::new(self.relay.with_policies(inputs.policies));
		self
	}

	/// delete any active sessions
	pub async fn delete_session(&self, parts: Parts) -> Result<Response, ProxyError> {
		let req_uri = parts.uri.clone();
		let ctx = IncomingRequestContext::new(&parts);
		let (_span, log, _cel) = mcp::relay::setup_request_log(parts, "delete_session");
		let session_id = self.id.to_string();
		log.non_atomic_mutate(|l| {
			// NOTE: l.method_name keep None to respect the metrics logic: not handle GET, DELETE.
			l.session_id = Some(session_id);
		});
		self.replay.shutdown().await;
		Self::handle_error(
			None,
			Some(req_uri),
			self.relay.send_fanout_deletion(ctx).await,
		)
		.await
	}

	/// forward_legacy_sse takes an upstream Response and forwards all messages to the SSE data stream.
	/// In SSE, POST requests always just get a 202 response and the messages go on a separate stream.
	/// Note: its plausible we could rewrite the rest of the proxy to return a more structured type than
	/// `Response` here, so we don't have to re-process it. However, since SSE is deprecated its best to
	/// optimize for the non-deprecated code paths; this works fine.
	pub async fn forward_legacy_sse(&self, resp: Response) -> Result<(), ClientError> {
		let Some(tx) = self.tx.clone() else {
			return Err(ClientError::new(anyhow!(
				"may only be called for SSE streams",
			)));
		};
		let content_type = resp.headers().get(CONTENT_TYPE);
		let sse = match content_type {
			Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
				trace!("forward SSE got SSE stream response");
				let content_encoding = resp.headers().typed_get::<headers::ContentEncoding>();
				let (body, _encoding) =
					crate::http::compression::decompress_body(resp.into_body(), content_encoding.as_ref())
						.map_err(ClientError::new)?;
				let event_stream = SseStream::from_byte_stream(body.into_data_stream()).boxed();
				StreamableHttpPostResponse::Sse(event_stream, None)
			},
			Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
				trace!("forward SSE got single JSON response");
				let message = json::from_response_body::<ServerJsonRpcMessage>(resp)
					.await
					.map_err(ClientError::new)?;
				StreamableHttpPostResponse::Json(message, None)
			},
			_ => {
				trace!("forward SSE got accepted, no action needed");
				return Ok(());
			},
		};
		let mut ms: Messages = sse.try_into()?;
		tokio::spawn(async move {
			while let Some(Ok(msg)) = ms.next().await {
				let Ok(()) = tx.send(msg).await else {
					return;
				};
			}
		});
		Ok(())
	}

	/// get_stream establishes a stream for server-sent messages
	pub async fn get_stream(&self, parts: Parts) -> Result<Response, ProxyError> {
		let req_uri = parts.uri.clone();
		let ctx = IncomingRequestContext::new(&parts);
		let last_event_id = parts
			.headers
			.get(LAST_EVENT_ID_HEADER)
			.and_then(|value| value.to_str().ok())
			.map(str::to_owned);
		let (_span, log, _cel) = mcp::relay::setup_request_log(parts, "get_stream");
		let session_id = self.id.to_string();
		log.non_atomic_mutate(|l| {
			// NOTE: l.method_name keep None to respect the metrics logic: which do not want to handle GET, DELETE.
			l.session_id = Some(session_id);
		});
		let replay_session_id = self.id.to_string();
		let response = self
			.replay
			.open_stream(self.relay.clone(), ctx, last_event_id.as_deref())
			.await
			.map(|subscription| {
				let replay = futures_util::stream::iter(subscription.replay);
				let live = futures_util::stream::unfold(subscription.receiver, {
					let replay_session_id = replay_session_id.clone();
					move |mut receiver| {
						let replay_session_id = replay_session_id.clone();
						async move {
							match receiver.recv().await {
								Ok(message) => Some((message, receiver)),
								Err(broadcast::error::RecvError::Closed) => None,
								Err(broadcast::error::RecvError::Lagged(skipped)) => {
									tracing::warn!(
										skipped,
										session_id = %replay_session_id,
										"MCP replay subscriber lagged behind the local event buffer; closing stream"
									);
									None
								},
							}
						}
					}
				});
				sse_stream_response(replay.chain(live), None)
			});
		Self::handle_error(None, Some(req_uri), response).await
	}

	async fn handle_error(
		req_id: Option<RequestId>,
		request_uri: Option<Uri>,
		d: Result<Response, UpstreamError>,
	) -> Result<Response, ProxyError> {
		match d {
			Ok(r) => Ok(r),
			Err(UpstreamError::Http(ClientError::Status(resp))) => {
				let mut response = *resp;
				if let Some(request_uri) = request_uri.as_ref()
					&& let Err(e) =
						crate::mcp::rewrite_www_authenticate_for_request_uri(&mut response, request_uri)
				{
					tracing::warn!(
						"failed to rewrite mcp upstream WWW-Authenticate header: {}",
						e
					);
				}
				let resp = http::SendDirectResponse::new(response)
					.await
					.map_err(ProxyError::Body)?;
				Err(mcp::Error::UpstreamError(Box::new(resp)).into())
			},
			Err(UpstreamError::Proxy(p)) => Err(p),
			Err(UpstreamError::Authorization {
				resource_type,
				resource_name,
			}) if req_id.is_some() => Self::json_rpc_error_response(
				req_id,
				ErrorData::invalid_params(format!("Unknown {resource_type}: {resource_name}"), None),
			),
			Err(UpstreamError::InvalidRequest(msg)) => {
				Self::json_rpc_error_response(req_id, ErrorData::invalid_request(msg, None))
			},
			Err(UpstreamError::InvalidMethod(msg)) => Self::json_rpc_error_response(
				req_id,
				ErrorData::new(ErrorCode::METHOD_NOT_FOUND, msg, None),
			),
			Err(UpstreamError::InvalidMethodWithMultiplexing(msg)) => {
				Self::json_rpc_error_response(req_id, ErrorData::invalid_request(msg, None))
			},
			Err(UpstreamError::ServiceError(err)) => match err {
				rmcp::ServiceError::McpError(data) => Self::json_rpc_error_response(req_id, data),
				other => {
					Self::json_rpc_error_response(req_id, ErrorData::internal_error(other.to_string(), None))
				},
			},
			Err(UpstreamError::Http(err)) => {
				Self::json_rpc_error_response(req_id, ErrorData::internal_error(err.to_string(), None))
			},
			Err(UpstreamError::OpenAPIError(err)) => {
				Self::json_rpc_error_response(req_id, ErrorData::internal_error(err.to_string(), None))
			},
			Err(UpstreamError::Stdio(err)) => {
				Self::json_rpc_error_response(req_id, ErrorData::internal_error(err.to_string(), None))
			},
			Err(UpstreamError::Send) => Self::json_rpc_error_response(
				req_id,
				ErrorData::internal_error("upstream closed on send".to_string(), None),
			),
			Err(UpstreamError::Authorization {
				resource_type,
				resource_name,
			}) => Err(
				mcp::Error::SendError(req_id, format!("unknown {resource_type}: {resource_name}")).into(),
			),
		}
	}

	fn json_rpc_error_response(
		req_id: Option<RequestId>,
		error: ErrorData,
	) -> Result<Response, ProxyError> {
		let Some(id) = req_id else {
			return Err(mcp::Error::SendError(None, error.to_string()).into());
		};
		let body = serde_json::to_string(&JsonRpcError {
			jsonrpc: Default::default(),
			id,
			error,
		})
		.unwrap_or_else(|_| "{\"error\":\"failed to serialize jsonrpc error\"}".to_string());
		Ok(
			::http::Response::builder()
				.status(StatusCode::OK)
				.header(CONTENT_TYPE, JSON_MIME_TYPE)
				.body(crate::http::Body::from(body))
				.expect("valid response"),
		)
	}

	fn resolve_resource_uri_for_upstream(
		&self,
		method: &str,
		uri: &mut String,
	) -> Result<String, UpstreamError> {
		let Some((service_name, original_uri)) = self.relay.unwrap_resource_uri(uri.as_str()) else {
			return Err(UpstreamError::InvalidMethodWithMultiplexing(
				method.to_string(),
			));
		};
		*uri = original_uri;
		Ok(service_name)
	}

	fn route_resource_to_upstream(
		&self,
		method: &str,
		uri: &mut String,
		log: &crate::telemetry::log::AsyncLog<mcp::MCPInfo>,
		cel: &rbac::CelExecWrapper,
	) -> Result<String, UpstreamError> {
		let service_name = self.resolve_resource_uri_for_upstream(method, uri)?;
		let resource_name = uri.clone();
		log.non_atomic_mutate(|l| {
			l.target_name = Some(service_name.clone());
			l.resource_name = Some(resource_name.clone());
			l.resource = Some(MCPOperation::Resource);
		});
		if !self.relay.policies.validate(
			&rbac::ResourceType::Resource(rbac::ResourceId::new(
				service_name.as_str(),
				resource_name.as_str(),
			)),
			cel,
		) {
			return Err(UpstreamError::Authorization {
				resource_type: "resource".to_string(),
				resource_name,
			});
		}
		Ok(service_name)
	}

	fn route_task_to_upstream(
		&self,
		task_id: &mut String,
		log: &crate::telemetry::log::AsyncLog<mcp::MCPInfo>,
		cel: &rbac::CelExecWrapper,
	) -> Result<String, UpstreamError> {
		let original_task_id = task_id.clone();
		let (service_name, task) = self.relay.parse_resource_name(task_id.as_str())?;
		let service_name = service_name.to_string();
		let task_name = task.to_string();
		*task_id = task_name.clone();
		log.non_atomic_mutate(|l| {
			l.target_name = Some(service_name.clone());
			l.resource_name = Some(task_name.clone());
			l.resource = Some(MCPOperation::Task);
		});
		if !self.relay.policies.validate(
			&rbac::ResourceType::Task(rbac::ResourceId::new(
				service_name.as_str(),
				task_name.as_str(),
			)),
			cel,
		) {
			return Err(UpstreamError::Authorization {
				resource_type: "task".to_string(),
				resource_name: original_task_id,
			});
		}
		Ok(service_name)
	}

	fn multiplex_target_names(&self, get: impl FnOnce(&Relay) -> Vec<Strng>) -> Option<Vec<Strng>> {
		self
			.relay
			.is_multiplexing()
			.then(|| get(self.relay.as_ref()))
	}

	async fn send_fanout_maybe_targeted(
		&self,
		request: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
		target_names: Option<Vec<Strng>>,
	) -> Result<Response, UpstreamError> {
		if let Some(target_names) = target_names {
			self
				.relay
				.send_fanout_to(request, ctx, merge, target_names)
				.await
		} else {
			self.relay.send_fanout(request, ctx, merge).await
		}
	}

	async fn send_internal(
		&mut self,
		parts: Parts,
		message: ClientJsonRpcMessage,
	) -> Result<Response, UpstreamError> {
		// Sending a message entails fanning out the message to each upstream, and then aggregating the responses.
		// The responses may include any number of notifications on the same HTTP response, and then finish with the
		// response to the request.
		// To merge these, we use a MergeStream which will join all of the notifications together, and then apply
		// some per-request merge logic across all the responses.
		// For example, this may return [server1-notification, server2-notification, server2-notification, merge(server1-response, server2-response)].
		// It's very common to not have any notifications, though.
		match message {
			ClientJsonRpcMessage::Request(mut r) => {
				let method = r.request.method().to_string();
				let ctx = IncomingRequestContext::new(&parts);
				let (_span, log, cel) = mcp::relay::setup_request_log(parts, &method);
				let session_id = self.id.to_string();
				log.non_atomic_mutate(|l| {
					l.method_name = Some(method.clone());
					l.session_id = Some(session_id);
				});
				match &mut r.request {
					ClientRequest::InitializeRequest(ir) => {
						if self.relay.count() == 0 {
							return Err(UpstreamError::Proxy(
								mcp::Error::SendError(None, "no MCP targets available".to_string()).into(),
							));
						}
						let pv = ir.params.protocol_version.clone();
						// Mint the resumable session token only after initialize has
						// fully succeeded and the relay has captured the negotiated
						// membership + initialize metadata for this session.
						let res = self.relay.send_initialize(r, ctx, pv).await;
						if res.is_ok() {
							match self.relay.snapshot_state() {
								Ok(state) => {
									let s = http::sessionpersistence::SessionState::MCPSnapshot(state);
									match s.encode(&self.encoder) {
										Ok(id) => {
											self.id = id.into();
											self.relay.set_session_binding(self.id.as_ref());
											self.replay.set_session_binding(self.id.as_ref());
										},
										Err(e) => {
											if self.continuity == SessionContinuity::Reconstructible {
												self.downgrade_continuity(SessionContinuity::LiveOnly);
											}
											tracing::warn!(error = %e, "failed to encode mcp session snapshot");
										},
									}
								},
								Err(e) => {
									if self.continuity == SessionContinuity::Reconstructible {
										self.downgrade_continuity(SessionContinuity::LiveOnly);
									}
									tracing::warn!(error = %e, "failed to mint resumable mcp session snapshot");
								},
							}
						}
						res
					},
					ClientRequest::ListToolsRequest(_) => {
						log.non_atomic_mutate(|l| {
							l.resource = Some(MCPOperation::Tool);
						});
						let names = self.multiplex_target_names(Relay::upstreams_with_tools);
						self
							.send_fanout_maybe_targeted(r, ctx, self.relay.merge_tools(cel), names)
							.await
					},
					ClientRequest::PingRequest(_) => {
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_empty())
							.await
					},
					ClientRequest::SetLevelRequest(_) => {
						let names = self.multiplex_target_names(Relay::upstreams_with_logging);
						self
							.send_fanout_maybe_targeted(r, ctx, self.relay.merge_empty(), names)
							.await
					},
					ClientRequest::ListPromptsRequest(_) => {
						log.non_atomic_mutate(|l| {
							l.resource = Some(MCPOperation::Prompt);
						});
						let names = self.multiplex_target_names(Relay::upstreams_with_prompts);
						self
							.send_fanout_maybe_targeted(r, ctx, self.relay.merge_prompts(cel), names)
							.await
					},
					ClientRequest::ListResourcesRequest(_) => {
						log.non_atomic_mutate(|l| {
							l.resource = Some(MCPOperation::Resource);
						});
						let names = self.multiplex_target_names(Relay::upstreams_with_resources);
						self
							.send_fanout_maybe_targeted(r, ctx, self.relay.merge_resources(cel), names)
							.await
					},
					ClientRequest::ListResourceTemplatesRequest(_) => {
						log.non_atomic_mutate(|l| {
							l.resource = Some(MCPOperation::ResourceTemplates);
						});
						let names = self.multiplex_target_names(Relay::upstreams_with_resources);
						self
							.send_fanout_maybe_targeted(r, ctx, self.relay.merge_resource_templates(cel), names)
							.await
					},
					ClientRequest::CallToolRequest(ctr) => {
						let name = ctr.params.name.clone();
						let (service_name, tool) = self.relay.parse_resource_name(&name)?;
						log.non_atomic_mutate(|l| {
							l.resource_name = Some(tool.to_string());
							l.target_name = Some(service_name.to_string());
							l.resource = Some(MCPOperation::Tool);
						});
						if !self.relay.policies.validate(
							&rbac::ResourceType::Tool(rbac::ResourceId::new(service_name, tool)),
							&cel,
						) {
							return Err(UpstreamError::Authorization {
								resource_type: "tool".to_string(),
								resource_name: name.to_string(),
							});
						}

						let tn = tool.to_string();
						ctr.params.name = tn.into();
						self.relay.send_single(r, ctx, service_name).await
					},
					ClientRequest::GetPromptRequest(gpr) => {
						let name = gpr.params.name.clone();
						let (service_name, prompt) = self.relay.parse_resource_name(&name)?;
						log.non_atomic_mutate(|l| {
							l.target_name = Some(service_name.to_string());
							l.resource_name = Some(prompt.to_string());
							l.resource = Some(MCPOperation::Prompt);
						});
						if !self.relay.policies.validate(
							&rbac::ResourceType::Prompt(rbac::ResourceId::new(service_name, prompt)),
							&cel,
						) {
							return Err(UpstreamError::Authorization {
								resource_type: "prompt".to_string(),
								resource_name: name.to_string(),
							});
						}
						gpr.params.name = prompt.to_string();
						self.relay.send_single(r, ctx, service_name).await
					},
					ClientRequest::ReadResourceRequest(rrr) => {
						let service_name =
							self.route_resource_to_upstream(&method, &mut rrr.params.uri, &log, &cel)?;
						self.relay.send_single(r, ctx, &service_name).await
					},
					ClientRequest::ListTasksRequest(_) => {
						log.non_atomic_mutate(|l| {
							l.resource = Some(MCPOperation::Task);
						});
						let names = self.multiplex_target_names(Relay::upstreams_with_tasks);
						self
							.send_fanout_maybe_targeted(r, ctx, self.relay.merge_tasks(cel), names)
							.await
					},
					ClientRequest::GetTaskInfoRequest(gti) => {
						let service_name = self.route_task_to_upstream(&mut gti.params.task_id, &log, &cel)?;
						self.relay.send_single(r, ctx, &service_name).await
					},
					ClientRequest::GetTaskResultRequest(gtr) => {
						let service_name = self.route_task_to_upstream(&mut gtr.params.task_id, &log, &cel)?;
						self.relay.send_single(r, ctx, &service_name).await
					},
					ClientRequest::CancelTaskRequest(ctr) => {
						let service_name = self.route_task_to_upstream(&mut ctr.params.task_id, &log, &cel)?;
						self.relay.send_single(r, ctx, &service_name).await
					},
					ClientRequest::SubscribeRequest(sr) => {
						let service_name =
							self.route_resource_to_upstream(&method, &mut sr.params.uri, &log, &cel)?;
						self.relay.send_single(r, ctx, &service_name).await
					},
					ClientRequest::UnsubscribeRequest(usr) => {
						let service_name =
							self.route_resource_to_upstream(&method, &mut usr.params.uri, &log, &cel)?;
						self.relay.send_single(r, ctx, &service_name).await
					},
					ClientRequest::CustomRequest(_) => {
						// TODO(https://github.com/agentgateway/agentgateway/issues/404)
						Err(UpstreamError::InvalidMethod(r.request.method().to_string()))
					},
					ClientRequest::CompleteRequest(cr) => {
						use rmcp::model::Reference;
						let (service_name, resource_type, original_id, operation) = match &cr.params.r#ref {
							Reference::Resource(rr) => {
								let (service_name, original_uri) =
									self.relay.unwrap_resource_uri(&rr.uri).ok_or_else(|| {
										UpstreamError::InvalidRequest("invalid resource uri".to_string())
									})?;
								(
									service_name.clone(),
									rbac::ResourceType::Resource(rbac::ResourceId::new(
										service_name.as_str(),
										original_uri.as_str(),
									)),
									original_uri,
									MCPOperation::Resource,
								)
							},
							Reference::Prompt(pr) => {
								let (service_name, original_name) = self.relay.parse_resource_name(&pr.name)?;
								let service_name_str = service_name.to_string();
								(
									service_name_str.clone(),
									rbac::ResourceType::Prompt(rbac::ResourceId::new(service_name, original_name)),
									original_name.to_string(),
									MCPOperation::Prompt,
								)
							},
						};

						log.non_atomic_mutate(|l| {
							l.target_name = Some(service_name.clone());
							l.resource_name = Some(original_id.clone());
							l.resource = Some(operation);
						});

						if !self.relay.policies.validate(&resource_type, &cel) {
							return Err(UpstreamError::Authorization {
								resource_type: match resource_type {
									rbac::ResourceType::Resource(_) => "resource".to_string(),
									rbac::ResourceType::Prompt(_) => "prompt".to_string(),
									rbac::ResourceType::Task(_) => "task".to_string(),
									_ => "unknown".to_string(),
								},
								resource_name: original_id,
							});
						}

						match &mut cr.params.r#ref {
							Reference::Resource(rr) => rr.uri = original_id,
							Reference::Prompt(pr) => pr.name = original_id,
						}

						self.relay.send_single(r, ctx, &service_name).await
					},
				}
			},
			ClientJsonRpcMessage::Notification(r) => {
				let method = match &r.notification {
					ClientNotification::CancelledNotification(r) => r.method.as_str(),
					ClientNotification::ProgressNotification(r) => r.method.as_str(),
					ClientNotification::InitializedNotification(r) => r.method.as_str(),
					ClientNotification::RootsListChangedNotification(r) => r.method.as_str(),
					ClientNotification::CustomNotification(r) => r.method.as_str(),
				};
				let ctx = IncomingRequestContext::new(&parts);
				let (_span, log, _cel) = mcp::relay::setup_request_log(parts, method);
				let session_id = self.id.to_string();
				log.non_atomic_mutate(|l| {
					l.method_name = Some(method.to_string());
					l.session_id = Some(session_id);
				});
				// Relay routes encoded upstream identifiers back to the correct target and
				// fans out the rest. Target-local lifecycle checks, like pending
				// elicitation tracking, stay in TargetSession under the relay.
				self.relay.send_notification(r, ctx).await
			},
			ClientJsonRpcMessage::Response(mut r) => {
				let ctx = IncomingRequestContext::new(&parts);
				let (service_name, id) = self.relay.decode_upstream_request_id(&r.id)?;
				r.id = id;
				self
					.relay
					.send_client_message(service_name, ClientJsonRpcMessage::Response(r), ctx)
					.await
			},
			ClientJsonRpcMessage::Error(mut r) => {
				let ctx = IncomingRequestContext::new(&parts);
				let (service_name, id) = self.relay.decode_upstream_request_id(&r.id)?;
				r.id = id;
				self
					.relay
					.send_client_message(service_name, ClientJsonRpcMessage::Error(r), ctx)
					.await
			},
		}
	}
}

#[derive(Debug)]
pub struct SessionManager {
	encoder: http::sessionpersistence::Encoder,
	instance_id: Arc<str>,
	sessions: RwLock<HashMap<String, Session>>,
}

fn local_session_id() -> Arc<str> {
	uuid::Uuid::new_v4().to_string().into()
}

impl SessionManager {
	pub fn new(encoder: http::sessionpersistence::Encoder) -> Self {
		Self {
			encoder,
			instance_id: uuid::Uuid::new_v4().to_string().into(),
			sessions: Default::default(),
		}
	}

	fn ensure_encrypted_session_ids_for_routed_identifiers(
		&self,
		uses_routed_identifiers: bool,
		allow_insecure_multiplex: bool,
	) -> Result<(), http::sessionpersistence::Error> {
		if uses_routed_identifiers && !self.encoder.is_encrypted() && !allow_insecure_multiplex {
			return Err(http::sessionpersistence::Error::EncryptedSessionIdsRequired);
		}
		if uses_routed_identifiers && !self.encoder.is_encrypted() && allow_insecure_multiplex {
			warn!(
				"allow_insecure_multiplex is enabled for this backend; routed MCP identifiers will use insecure base64 encoding"
			);
		}
		Ok(())
	}

	fn encode_instance_bound_session_id(
		&self,
		local_id: &str,
	) -> Result<Arc<str>, http::sessionpersistence::Error> {
		let state = http::sessionpersistence::SessionState::MCPInstanceRef(
			http::sessionpersistence::MCPInstanceRefState::new(self.instance_id.as_ref(), local_id),
		);
		state.encode(&self.encoder).map(Into::into)
	}

	pub fn get_session(&self, id: &str, builder: RelayInputs) -> Option<Session> {
		let session = self
			.sessions
			.read()
			.ok()?
			.get(id)
			.cloned()?
			.with_inputs(builder);
		tracing::debug!(
			continuity = %session.continuity(),
			"loaded live MCP session"
		);
		Some(session)
	}

	pub fn resolve_session(
		&self,
		id: &str,
		builder: RelayInputs,
	) -> Result<Session, ResumeFailureReason> {
		if let Some(s) = self.sessions.read().expect("poisoned").get(id).cloned() {
			let session = s.with_inputs(builder);
			tracing::debug!(
				continuity = %session.continuity(),
				"reused live MCP session"
			);
			return Ok(session);
		}
		let allow_insecure_multiplex = builder.backend.allow_insecure_multiplex;
		let d = match http::sessionpersistence::SessionState::decode(id, &self.encoder) {
			Ok(state) => state,
			Err(_) => return Err(ResumeFailureReason::MalformedHandle),
		};
		let state = match d {
			http::sessionpersistence::SessionState::MCPSnapshot(state) => state,
			http::sessionpersistence::SessionState::MCPInstanceRef(state) => {
				// Instance refs only tell us which pod minted the handle. They are
				// enough to report "live session missing", but never to reconstruct.
				let same_instance = state.instance_id == self.instance_id.as_ref();
				tracing::debug!(
					same_instance,
					reason = %ResumeFailureReason::LiveSessionMissing,
					"instance-bound MCP session handle cannot be resumed without a live local session"
				);
				return Err(ResumeFailureReason::LiveSessionMissing);
			},
			http::sessionpersistence::SessionState::HTTP(_) => {
				return Err(ResumeFailureReason::UnsupportedHandle);
			},
		};
		if self
			.ensure_encrypted_session_ids_for_routed_identifiers(
				state.routing.default_target_name.is_none(),
				allow_insecure_multiplex,
			)
			.is_err()
		{
			warn!(
				reason = %ResumeFailureReason::EncryptedSessionIdsRequired,
				"failed to resume session: routed MCP identifiers require encrypted session ids"
			);
			return Err(ResumeFailureReason::EncryptedSessionIdsRequired);
		}
		let member_names = state
			.members
			.iter()
			.map(|member| member.target.as_str())
			.collect::<Vec<_>>()
			.join(", ");
		let Some(relay) = builder
			.build_snapshot_connections(&state.members, state.routing)
			.map_err(|error| {
				warn!(
					reason = %ResumeFailureReason::SnapshotRestoreFailed,
					error = %error,
					"failed to rebuild session snapshot connections"
				);
				ResumeFailureReason::SnapshotRestoreFailed
			})?
		else {
			warn!(
				reason = %ResumeFailureReason::SnapshotMismatch,
				"failed to resume session: snapshot members [{member_names}] no longer match config"
			);
			return Err(ResumeFailureReason::SnapshotMismatch);
		};
		if let Err(e) = relay.restore_snapshot_state(&state.members) {
			warn!(
				reason = %ResumeFailureReason::SnapshotRestoreFailed,
				error = %e,
				"failed to restore snapshot state"
			);
			return Err(ResumeFailureReason::SnapshotRestoreFailed);
		}

		let sess = Session {
			id: id.into(),
			relay: Arc::new(relay.with_session_binding(id, self.encoder.clone())),
			replay: {
				let replay = SessionReplayState::new(self.encoder.clone());
				replay.set_session_binding(id);
				replay
			},
			tx: None,
			encoder: self.encoder.clone(),
			continuity: SessionContinuity::Reconstructible,
		};
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(id.to_string(), sess.clone());
		tracing::debug!(
			continuity = %sess.continuity(),
			"reconstructed MCP session from snapshot"
		);
		Ok(sess)
	}

	/// create_session establishes an MCP session.
	pub fn create_session(&self, relay: Relay) -> Result<Session, http::sessionpersistence::Error> {
		self.ensure_encrypted_session_ids_for_routed_identifiers(
			relay.uses_routed_identifiers(),
			relay.allow_insecure_multiplex(),
		)?;
		let id = self.encode_instance_bound_session_id(local_session_id().as_ref())?;
		let relay = relay.with_session_binding(id.as_ref(), self.encoder.clone());
		let continuity = relay.session_continuity();

		// Do NOT insert yet
		Ok(Session {
			id: id.clone(),
			relay: Arc::new(relay),
			replay: {
				let replay = SessionReplayState::new(self.encoder.clone());
				replay.set_session_binding(id.as_ref());
				replay
			},
			tx: None,
			encoder: self.encoder.clone(),
			continuity,
		})
	}

	pub fn insert_session(&self, sess: Session) {
		tracing::debug!(
			continuity = %sess.continuity(),
			"registered MCP session"
		);
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(sess.id.to_string(), sess);
	}

	/// create_stateless_session creates a session for stateless mode.
	/// Unlike create_session, this does NOT register the session in the session manager.
	/// The caller is responsible for calling session.delete_session() when done
	/// to clean up upstream resources (e.g., stdio processes).
	pub fn create_stateless_session(
		&self,
		relay: Relay,
	) -> Result<Session, http::sessionpersistence::Error> {
		self.ensure_encrypted_session_ids_for_routed_identifiers(
			relay.uses_routed_identifiers(),
			relay.allow_insecure_multiplex(),
		)?;
		let id = local_session_id();
		let session = Session {
			id: id.clone(),
			relay: Arc::new(relay.with_session_binding(id.as_ref(), self.encoder.clone())),
			replay: {
				let replay = SessionReplayState::new(self.encoder.clone());
				replay.set_session_binding(id.as_ref());
				replay
			},
			tx: None,
			encoder: self.encoder.clone(),
			continuity: SessionContinuity::OneShot,
		};
		tracing::debug!(
			continuity = %session.continuity(),
			"created one-shot MCP session"
		);
		Ok(session)
	}

	/// create_legacy_session establishes a legacy SSE session.
	/// These will have the ability to send messages to them via a channel.
	pub fn create_legacy_session(
		&self,
		relay: Relay,
	) -> Result<(Session, Receiver<ServerJsonRpcMessage>), http::sessionpersistence::Error> {
		self.ensure_encrypted_session_ids_for_routed_identifiers(
			relay.uses_routed_identifiers(),
			relay.allow_insecure_multiplex(),
		)?;
		let (tx, rx) = tokio::sync::mpsc::channel(64);
		let id = self.encode_instance_bound_session_id(local_session_id().as_ref())?;
		let sess = Session {
			id: id.clone(),
			relay: Arc::new(relay.with_session_binding(id.as_ref(), self.encoder.clone())),
			replay: {
				let replay = SessionReplayState::new(self.encoder.clone());
				replay.set_session_binding(id.as_ref());
				replay
			},
			tx: Some(tx),
			encoder: self.encoder.clone(),
			continuity: SessionContinuity::LiveOnly,
		};
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(id.to_string(), sess.clone());
		tracing::debug!(
			continuity = %sess.continuity(),
			"created legacy SSE MCP session"
		);
		Ok((sess, rx))
	}

	pub async fn delete_session(&self, id: &str, parts: Parts) -> Option<Response> {
		let sess = {
			let mut sm = self.sessions.write().expect("write lock");
			sm.remove(id)?
		};
		tracing::debug!(
			continuity = %sess.continuity(),
			"deleted MCP session"
		);
		// Swallow the error
		sess.delete_session(parts).await.ok()
	}

	#[cfg(test)]
	fn contains_session(&self, id: &str) -> bool {
		self.sessions.read().expect("read lock").contains_key(id)
	}
}

#[derive(Debug, Clone)]
pub struct SessionDropper {
	sm: Arc<SessionManager>,
	s: Option<(Session, Parts)>,
}

/// Dropper returns a handle that, when dropped, removes the session
pub fn dropper(sm: Arc<SessionManager>, s: Session, parts: Parts) -> SessionDropper {
	SessionDropper {
		sm,
		s: Some((s, parts)),
	}
}

impl SessionDropper {
	fn take(&mut self) -> Option<(Session, Parts)> {
		self.s.take()
	}

	pub async fn cleanup(mut self) {
		let Some((s, parts)) = self.take() else {
			return;
		};
		self
			.sm
			.sessions
			.write()
			.expect("write lock")
			.remove(s.id.as_ref());
		let _ = s.delete_session(parts).await;
	}
}

impl Drop for SessionDropper {
	fn drop(&mut self) {
		let Some((s, parts)) = self.take() else {
			return;
		};
		let mut sm = self.sm.sessions.write().expect("write lock");
		debug!(continuity = %s.continuity(), "dropping MCP session");
		sm.remove(s.id.as_ref());
		tokio::task::spawn(async move { s.delete_session(parts).await });
	}
}

pub(crate) fn sse_stream_response(
	stream: impl futures::Stream<Item = ServerSseMessage> + Send + 'static,
	keep_alive: Option<Duration>,
) -> Response {
	use futures::StreamExt;
	let stream = SseBody::new(stream.map(|message| {
		let data = serde_json::to_string(&message.message).expect("valid message");
		let mut sse = Sse::default().data(data);
		sse.id = message.event_id;
		Result::<Sse, Infallible>::Ok(sse)
	}));
	let stream = match keep_alive {
		Some(duration) => {
			http::Body::new(stream.with_keep_alive::<TokioSseTimer>(KeepAlive::new().interval(duration)))
		},
		None => http::Body::new(stream),
	};
	::http::Response::builder()
		.status(StatusCode::OK)
		.header(http::header::CONTENT_TYPE, EVENT_STREAM_MIME_TYPE)
		.header(http::header::CACHE_CONTROL, "no-cache")
		.body(stream)
		.expect("valid response")
}

pin_project_lite::pin_project! {
		struct TokioSseTimer {
				#[pin]
				sleep: tokio::time::Sleep,
		}
}
impl Future for TokioSseTimer {
	type Output = ();

	fn poll(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Self::Output> {
		let this = self.project();
		this.sleep.poll(cx)
	}
}
impl sse_stream::Timer for TokioSseTimer {
	fn from_duration(duration: Duration) -> Self {
		Self {
			sleep: tokio::time::sleep(duration),
		}
	}

	fn reset(self: std::pin::Pin<&mut Self>, when: std::time::Instant) {
		let this = self.project();
		this.sleep.reset(tokio::time::Instant::from_std(when));
	}
}

fn get_client_info(protocol_version: ProtocolVersion) -> ClientInfo {
	ClientInfo::new(
		rmcp::model::ClientCapabilities::default(),
		Implementation::new("agentgateway", BuildInfo::new().version.to_string()),
	)
	.with_protocol_version(protocol_version)
}

fn protocol_version_from_headers(
	headers: &http::HeaderMap,
) -> Result<ProtocolVersion, UpstreamError> {
	let Some(raw) = headers.get(HEADER_MCP_PROTOCOL_VERSION) else {
		return Err(UpstreamError::InvalidRequest(
			"missing MCP-Protocol-Version header for synthetic initialize; send initialize first"
				.to_string(),
		));
	};
	let Ok(value) = raw.to_str() else {
		return Err(UpstreamError::InvalidRequest(
			"invalid MCP-Protocol-Version header encoding".to_string(),
		));
	};
	// ProtocolVersion has no public constructor/FromStr, so deserialize from a JSON string value.
	serde_json::from_value::<ProtocolVersion>(serde_json::Value::String(value.to_owned())).map_err(
		|_| {
			UpstreamError::InvalidRequest(format!(
				"invalid MCP-Protocol-Version header value: {value}"
			))
		},
	)
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;
	use std::path::Path;
	use std::time::Duration;

	use super::*;
	use crate::http::sessionpersistence::{
		Encoder, MCPSession, MCPSnapshotMember, MCPSnapshotRouting, MCPSnapshotState, SessionState,
	};
	use crate::mcp::McpAuthorizationSet;
	use crate::mcp::router::{McpBackendGroup, McpTarget};
	use crate::mcp::upstream::UpstreamError;
	use crate::proxy::httpproxy::PolicyClient;
	use crate::types::agent::McpTargetSpec;
	use agent_core::strng::Strng;
	use rmcp::model::{
		JsonRpcNotification, PromptsCapability, ProtocolVersion, ResourceUpdatedNotification,
		ResourceUpdatedNotificationParam, ServerCapabilities, ServerInfo, ServerJsonRpcMessage,
		ServerNotification, ToolsCapability,
	};
	use serde_json::json;
	use tempfile::NamedTempFile;

	#[tokio::test]
	async fn handle_error_invalid_method_returns_jsonrpc() {
		let resp = Session::handle_error(
			Some(RequestId::Number(7)),
			None,
			Err(UpstreamError::InvalidMethod("nope".to_string())),
		)
		.await
		.expect("response");
		assert_eq!(resp.status(), StatusCode::OK);
		assert_eq!(
			resp
				.headers()
				.get(CONTENT_TYPE)
				.and_then(|v| v.to_str().ok()),
			Some(JSON_MIME_TYPE)
		);
		let body = crate::http::read_body_with_limit(resp.into_body(), 8 * 1024)
			.await
			.expect("body");
		let err: JsonRpcError = serde_json::from_slice(&body).expect("jsonrpc error");
		assert_eq!(err.id, RequestId::Number(7));
		assert_eq!(err.error.code, ErrorCode::METHOD_NOT_FOUND);
	}

	#[tokio::test]
	async fn handle_error_service_mcp_error_is_preserved() {
		let data = ErrorData::invalid_params("bad args", None);
		let resp = Session::handle_error(
			Some(RequestId::Number(9)),
			None,
			Err(UpstreamError::ServiceError(rmcp::ServiceError::McpError(
				data.clone(),
			))),
		)
		.await
		.expect("response");
		let body = crate::http::read_body_with_limit(resp.into_body(), 8 * 1024)
			.await
			.expect("body");
		let err: JsonRpcError = serde_json::from_slice(&body).expect("jsonrpc error");
		assert_eq!(err.error.code, data.code);
		assert_eq!(err.error.message, data.message);
	}

	#[tokio::test]
	async fn handle_error_authorization_returns_jsonrpc_invalid_params() {
		let resp = Session::handle_error(
			Some(RequestId::Number(13)),
			None,
			Err(UpstreamError::Authorization {
				resource_type: "tool".to_string(),
				resource_name: "echo".to_string(),
			}),
		)
		.await
		.expect("response");
		assert_eq!(resp.status(), StatusCode::OK);
		let body = crate::http::read_body_with_limit(resp.into_body(), 8 * 1024)
			.await
			.expect("body");
		let err: JsonRpcError = serde_json::from_slice(&body).expect("jsonrpc error");
		assert_eq!(err.id, RequestId::Number(13));
		assert_eq!(err.error.code, ErrorCode::INVALID_PARAMS);
		assert_eq!(err.error.message.as_ref(), "Unknown tool: echo");
	}

	#[tokio::test]
	async fn handle_error_http_passthrough_preserves_status() {
		let resp = ::http::Response::builder()
			.status(StatusCode::BAD_GATEWAY)
			.body(crate::http::Body::from("nope"))
			.expect("response");
		let err = Session::handle_error(
			Some(RequestId::Number(1)),
			None,
			Err(UpstreamError::Http(ClientError::Status(Box::new(resp)))),
		)
		.await
		.expect_err("expected error");
		match err {
			ProxyError::MCP(mcp::Error::UpstreamError(resp)) => {
				assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
			},
			other => panic!("unexpected error: {:?}", other),
		}
	}

	#[test]
	fn protocol_version_errors_when_header_missing() {
		let headers = http::HeaderMap::new();
		let err = protocol_version_from_headers(&headers).expect_err("header must be required");
		match err {
			UpstreamError::InvalidRequest(msg) => {
				assert!(msg.contains("missing MCP-Protocol-Version header"));
			},
			other => panic!("unexpected error: {other:?}"),
		}
	}

	#[test]
	fn protocol_version_uses_known_header_value() {
		let mut headers = http::HeaderMap::new();
		headers.insert(
			HEADER_MCP_PROTOCOL_VERSION,
			http::HeaderValue::from_static("2025-06-18"),
		);
		let pv = protocol_version_from_headers(&headers).expect("known version should parse");
		assert_eq!(pv.as_str(), "2025-06-18");
	}

	#[test]
	fn protocol_version_preserves_custom_header_value_for_passthrough() {
		let mut headers = http::HeaderMap::new();
		headers.insert(
			HEADER_MCP_PROTOCOL_VERSION,
			http::HeaderValue::from_static("2026-01-01"),
		);
		let pv = protocol_version_from_headers(&headers).expect("custom version should parse");
		assert_eq!(pv.as_str(), "2026-01-01");
	}

	#[test]
	fn protocol_version_errors_when_header_is_not_utf8() {
		let mut headers = http::HeaderMap::new();
		headers.insert(
			HEADER_MCP_PROTOCOL_VERSION,
			http::HeaderValue::from_bytes(b"\xFF").expect("non-utf8 header should be constructible"),
		);
		let err = protocol_version_from_headers(&headers).expect_err("header must be utf8");
		match err {
			UpstreamError::InvalidRequest(msg) => {
				assert_eq!(msg.as_str(), "invalid MCP-Protocol-Version header encoding");
			},
			other => panic!("unexpected error: {other:?}"),
		}
	}

	fn capture_target_with_prefix(
		name: &str,
		capture_file: &Path,
		always_use_prefix: bool,
	) -> Arc<McpTarget> {
		Arc::new(McpTarget {
			name: name.into(),
			spec: McpTargetSpec::Stdio {
				cmd: "sh".into(),
				args: vec![
					"-c".into(),
					"while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$CAPTURE_FILE\"; done".into(),
				],
				env: HashMap::from([(
					"CAPTURE_FILE".to_string(),
					capture_file.display().to_string(),
				)]),
			},
			backend: None,
			always_use_prefix,
			backend_policies: Default::default(),
		})
	}

	fn capture_target(name: &str, capture_file: &Path) -> Arc<McpTarget> {
		capture_target_with_prefix(name, capture_file, false)
	}

	fn relay_inputs_with_options(
		targets: Vec<Arc<McpTarget>>,
		allow_degraded: bool,
		allow_insecure_multiplex: bool,
	) -> crate::mcp::relay::RelayInputs {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		crate::mcp::relay::RelayInputs {
			backend: McpBackendGroup {
				targets,
				stateful: true,
				allow_degraded,
				allow_insecure_multiplex,
			},
			policies: McpAuthorizationSet::new(vec![].into()),
			client: PolicyClient {
				inputs: test.inputs(),
			},
		}
	}

	fn relay_inputs(
		targets: Vec<Arc<McpTarget>>,
		allow_degraded: bool,
	) -> crate::mcp::relay::RelayInputs {
		relay_inputs_with_options(targets, allow_degraded, false)
	}

	fn empty_parts() -> Parts {
		::http::Request::builder()
			.uri("/mcp")
			.body(crate::http::Body::empty())
			.expect("request should build")
			.into_parts()
			.0
	}

	fn resource_updated_message(uri: &str) -> ServerJsonRpcMessage {
		ServerJsonRpcMessage::notification(ServerNotification::ResourceUpdatedNotification(
			ResourceUpdatedNotification::new(ResourceUpdatedNotificationParam::new(uri)),
		))
	}

	fn resource_updated_uri(message: &ServerSseMessage) -> String {
		let ServerJsonRpcMessage::Notification(notification) = message.message.as_ref() else {
			panic!("expected notification message");
		};
		let ServerNotification::ResourceUpdatedNotification(update) = &notification.notification else {
			panic!("expected resource updated notification");
		};
		update.params.uri.clone()
	}

	async fn wait_for_replay_buffer_len(replay: &SessionReplayState, expected: usize) {
		let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
		loop {
			let len = replay.inner.lock().await.buffer.len();
			if len >= expected {
				return;
			}
			assert!(
				tokio::time::Instant::now() < deadline,
				"timed out waiting for {expected} replay messages, saw {len}"
			);
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	}

	#[tokio::test]
	async fn replay_subscription_replays_buffered_messages_after_last_event_id() {
		let replay = SessionReplayState::new(Encoder::base64());
		replay.set_session_binding("session-1");
		let messages = futures::stream::iter(vec![
			Ok(resource_updated_message("memo://one")),
			Ok(resource_updated_message("memo://two")),
			Ok(resource_updated_message("memo://three")),
		])
		.chain(futures::stream::pending());
		let _subscription = replay
			.start_test_stream(Messages::from_stream(messages))
			.await;

		wait_for_replay_buffer_len(&replay, 3).await;

		let first_event_id = {
			let inner = replay.inner.lock().await;
			inner
				.buffer
				.front()
				.and_then(|record| record.message.event_id.clone())
				.expect("buffered message should have an event id")
		};

		let subscription = replay
			.subscribe(Some(&first_event_id))
			.await
			.expect("subscription should succeed");

		let replayed_uris = subscription
			.replay
			.iter()
			.map(resource_updated_uri)
			.collect::<Vec<_>>();
		assert_eq!(replayed_uris, vec!["memo://two", "memo://three"]);

		replay.shutdown().await;
	}

	#[tokio::test]
	async fn replay_subscription_ignores_last_event_id_from_different_session_binding() {
		let first = SessionReplayState::new(Encoder::base64());
		first.set_session_binding("session-1");
		let first_messages = futures::stream::iter(vec![Ok(resource_updated_message("memo://one"))])
			.chain(futures::stream::pending());
		let _first_subscription = first
			.start_test_stream(Messages::from_stream(first_messages))
			.await;
		wait_for_replay_buffer_len(&first, 1).await;
		let foreign_event_id = {
			let inner = first.inner.lock().await;
			inner
				.buffer
				.front()
				.and_then(|record| record.message.event_id.clone())
				.expect("buffered message should have an event id")
		};

		let second = SessionReplayState::new(Encoder::base64());
		second.set_session_binding("session-2");
		let second_messages = futures::stream::iter(vec![
			Ok(resource_updated_message("memo://two")),
			Ok(resource_updated_message("memo://three")),
		])
		.chain(futures::stream::pending());
		let _second_subscription = second
			.start_test_stream(Messages::from_stream(second_messages))
			.await;
		wait_for_replay_buffer_len(&second, 2).await;

		let subscription = second
			.subscribe(Some(&foreign_event_id))
			.await
			.expect("subscription should succeed");

		assert!(
			subscription.replay.is_empty(),
			"foreign Last-Event-ID should start a fresh stream"
		);

		first.shutdown().await;
		second.shutdown().await;
	}

	#[tokio::test]
	async fn replay_subscription_receives_initial_messages_when_stream_starts() {
		let replay = SessionReplayState::new(Encoder::base64());
		replay.set_session_binding("session-1");
		let messages = futures::stream::iter(vec![Ok(resource_updated_message("memo://one"))])
			.chain(futures::stream::pending());
		let mut subscription = replay
			.start_test_stream(Messages::from_stream(messages))
			.await;

		let message = tokio::time::timeout(Duration::from_secs(1), subscription.receiver.recv())
			.await
			.expect("initial replay message should arrive before timeout")
			.expect("initial replay message should be available");

		assert_eq!(resource_updated_uri(&message), "memo://one");

		replay.shutdown().await;
	}

	fn encode_snapshot(
		session_encoder: &Encoder,
		members: Vec<MCPSnapshotMember>,
		routing: MCPSnapshotRouting,
	) -> String {
		SessionState::MCPSnapshot(MCPSnapshotState::new(members, routing))
			.encode(session_encoder)
			.expect("snapshot should encode")
	}

	fn server_info(has_tools: bool, has_prompts: bool) -> ServerInfo {
		let mut capabilities = ServerCapabilities::default();
		capabilities.tools = has_tools.then_some(ToolsCapability::default());
		capabilities.prompts = has_prompts.then_some(PromptsCapability::default());
		ServerInfo::new(capabilities).with_protocol_version(ProtocolVersion::V_2025_06_18)
	}

	fn snapshot_member(
		target: &Arc<McpTarget>,
		session: MCPSession,
		info: ServerInfo,
	) -> MCPSnapshotMember {
		MCPSnapshotMember::new(
			target.name.to_string(),
			session,
			info,
			target
				.snapshot_fingerprint()
				.expect("target fingerprint should be serializable"),
		)
	}

	async fn wait_until_contains(path: &Path, needle: &str) {
		let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
		loop {
			let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
			if content.contains(needle) {
				return;
			}
			if tokio::time::Instant::now() >= deadline {
				panic!(
					"capture {:?} did not contain {:?}; last content: {:?}",
					path, needle, content
				);
			}
			tokio::time::sleep(Duration::from_millis(20)).await;
		}
	}

	async fn assert_not_contains_for(path: &Path, needle: &str, duration: Duration) {
		let deadline = tokio::time::Instant::now() + duration;
		loop {
			let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
			if content.contains(needle) {
				panic!(
					"capture {:?} unexpectedly contained {:?}; content: {:?}",
					path, needle, content
				);
			}
			if tokio::time::Instant::now() >= deadline {
				return;
			}
			tokio::time::sleep(Duration::from_millis(20)).await;
		}
	}

	fn test_notification(method: &'static str) -> JsonRpcNotification<ClientNotification> {
		JsonRpcNotification {
			jsonrpc: Default::default(),
			notification: ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
				method,
				Some(json!({"ok": true})),
			)),
		}
	}

	fn encrypted_test_encoder() -> Encoder {
		Encoder::aes("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
			.expect("test codec should be valid")
	}

	#[tokio::test]
	async fn create_session_with_stdio_relay_is_live_only() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target("serverA", server_a_capture.path())],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let session_manager = SessionManager::new(Encoder::base64());
		let session = session_manager
			.create_session(relay)
			.expect("instance-bound session handle should mint");

		assert_eq!(session.continuity(), SessionContinuity::LiveOnly);

		let _ = session.delete_session(empty_parts()).await;
	}

	#[tokio::test]
	async fn create_stateless_session_is_one_shot() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target("serverA", server_a_capture.path())],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let session_manager = SessionManager::new(Encoder::base64());
		let session = session_manager
			.create_stateless_session(relay)
			.expect("one-shot session should create");

		assert_eq!(session.continuity(), SessionContinuity::OneShot);

		let _ = session.delete_session(empty_parts()).await;
	}

	#[tokio::test]
	async fn create_session_rejects_multiplex_without_encrypted_session_ids() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let session_manager = SessionManager::new(Encoder::base64());
		let err = session_manager
			.create_session(relay)
			.expect_err("multiplex sessions should require encrypted session ids");

		assert!(matches!(
			err,
			http::sessionpersistence::Error::EncryptedSessionIdsRequired
		));
	}

	#[tokio::test]
	async fn create_session_allows_multiplex_when_explicitly_opted_in() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs_with_options(
			vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			true,
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let session_manager = SessionManager::new(Encoder::base64());
		let session = session_manager
			.create_session(relay)
			.expect("explicit opt-in should allow insecure multiplex session ids");

		assert_eq!(session.continuity(), SessionContinuity::LiveOnly);

		let _ = session.delete_session(empty_parts()).await;
	}

	#[tokio::test]
	async fn create_session_rejects_single_target_routed_ids_without_encrypted_session_ids() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target_with_prefix(
				"serverA",
				server_a_capture.path(),
				true,
			)],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		assert!(
			relay.uses_routed_identifiers(),
			"single-target always_use_prefix should use routed identifiers"
		);
		let session_manager = SessionManager::new(Encoder::base64());
		let err = session_manager
			.create_session(relay)
			.expect_err("routed identifiers should require encrypted session ids");

		assert!(matches!(
			err,
			http::sessionpersistence::Error::EncryptedSessionIdsRequired
		));
	}

	#[tokio::test]
	async fn create_stateless_session_rejects_single_target_routed_ids_without_encrypted_session_ids()
	{
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target_with_prefix(
				"serverA",
				server_a_capture.path(),
				true,
			)],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let session_manager = SessionManager::new(Encoder::base64());
		let err = session_manager
			.create_stateless_session(relay)
			.expect_err("routed identifiers should require encrypted session ids");

		assert!(matches!(
			err,
			http::sessionpersistence::Error::EncryptedSessionIdsRequired
		));
	}

	#[tokio::test]
	async fn create_legacy_session_rejects_single_target_routed_ids_without_encrypted_session_ids() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target_with_prefix(
				"serverA",
				server_a_capture.path(),
				true,
			)],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let session_manager = SessionManager::new(Encoder::base64());
		let err = session_manager
			.create_legacy_session(relay)
			.expect_err("routed identifiers should require encrypted session ids");

		assert!(matches!(
			err,
			http::sessionpersistence::Error::EncryptedSessionIdsRequired
		));
	}

	#[tokio::test]
	async fn create_session_allows_single_target_routed_ids_when_explicitly_opted_in() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs_with_options(
			vec![capture_target_with_prefix(
				"serverA",
				server_a_capture.path(),
				true,
			)],
			true,
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let session_manager = SessionManager::new(Encoder::base64());
		let session = session_manager
			.create_session(relay)
			.expect("explicit opt-in should allow insecure routed identifiers");

		assert_eq!(session.continuity(), SessionContinuity::LiveOnly);

		let _ = session.delete_session(empty_parts()).await;
	}

	#[tokio::test]
	async fn create_legacy_session_is_live_only() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target("serverA", server_a_capture.path())],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let session_manager = SessionManager::new(Encoder::base64());
		let (session, _rx) = session_manager
			.create_legacy_session(relay)
			.expect("legacy session handle should mint");

		assert_eq!(session.continuity(), SessionContinuity::LiveOnly);

		let _ = session_manager
			.delete_session(session.id.as_ref(), empty_parts())
			.await;
	}

	#[tokio::test]
	async fn create_legacy_session_mints_instance_ref_handle() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target("serverA", server_a_capture.path())],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let encoder = encrypted_test_encoder();
		let session_manager = SessionManager::new(encoder.clone());
		let (session, _rx) = session_manager
			.create_legacy_session(relay)
			.expect("legacy session handle should mint");

		let decoded = SessionState::decode(session.id.as_ref(), &encoder)
			.expect("instance-bound handle should decode");
		assert!(matches!(decoded, SessionState::MCPInstanceRef(_)));

		let _ = session_manager
			.delete_session(session.id.as_ref(), empty_parts())
			.await;
	}

	#[tokio::test]
	async fn deleted_instance_bound_session_returns_none_not_invalid() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target("serverA", server_a_capture.path())],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let encoder = encrypted_test_encoder();
		let session_manager = SessionManager::new(encoder.clone());
		let (session, _rx) = session_manager
			.create_legacy_session(relay)
			.expect("legacy session handle should mint");
		let session_id = session.id.to_string();
		let _ = session_manager
			.delete_session(session_id.as_str(), empty_parts())
			.await;

		let err = session_manager
			.resolve_session(
				&session_id,
				relay_inputs(
					vec![capture_target("serverA", server_a_capture.path())],
					true,
				),
			)
			.expect_err("deleted instance-bound handle should classify as stale");

		assert_eq!(err, ResumeFailureReason::LiveSessionMissing);
	}

	#[tokio::test]
	async fn session_dropper_cleanup_removes_registered_session() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target("serverA", server_a_capture.path())],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let session_manager = Arc::new(SessionManager::new(Encoder::base64()));
		let session = session_manager
			.create_session(relay)
			.expect("session should create");
		let session_id = session.id.to_string();
		session_manager.insert_session(session.clone());
		assert!(session_manager.contains_session(&session_id));

		dropper(session_manager.clone(), session, empty_parts())
			.cleanup()
			.await;

		assert!(!session_manager.contains_session(&session_id));
	}

	#[tokio::test]
	async fn instance_bound_handle_from_other_manager_returns_none_not_invalid() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = relay_inputs(
			vec![capture_target("serverA", server_a_capture.path())],
			true,
		)
		.build_new_connections()
		.expect("relay should build");
		let encoder = encrypted_test_encoder();
		let origin_manager = SessionManager::new(encoder.clone());
		let (session, _rx) = origin_manager
			.create_legacy_session(relay)
			.expect("legacy session handle should mint");
		let session_id = session.id.to_string();
		let err = SessionManager::new(encoder)
			.resolve_session(
				&session_id,
				relay_inputs(
					vec![capture_target("serverA", server_a_capture.path())],
					true,
				),
			)
			.expect_err("cross-instance instance-bound handle should classify as stale");

		assert_eq!(err, ResumeFailureReason::LiveSessionMissing);
	}

	#[test]
	fn opaque_local_id_handle_is_malformed_when_not_live() {
		let encoder = encrypted_test_encoder();
		let session_manager = SessionManager::new(encoder);
		let opaque_local_id = uuid::Uuid::new_v4().to_string();

		let err = session_manager
			.resolve_session(&opaque_local_id, relay_inputs(Vec::new(), true))
			.expect_err("opaque local ids should classify as malformed when not live");

		assert_eq!(err, ResumeFailureReason::MalformedHandle);
	}

	#[test]
	fn resolve_session_returns_malformed_for_invalid_handle() {
		let session_manager = SessionManager::new(Encoder::base64());
		let err = session_manager
			.resolve_session(
				"definitely-not-a-session-handle",
				relay_inputs(Vec::new(), true),
			)
			.expect_err("garbage handle should classify as malformed");

		assert_eq!(err, ResumeFailureReason::MalformedHandle);
	}

	#[test]
	fn resolve_session_returns_snapshot_target_mismatch_for_missing_target() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let server_a = capture_target("serverA", server_a_capture.path());
		let server_b = capture_target("serverB", server_b_capture.path());
		let encoder = encrypted_test_encoder();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![
				snapshot_member(
					&server_a,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(true, false),
				),
				snapshot_member(
					&server_b,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(false, true),
				),
			],
			MCPSnapshotRouting {
				default_target_name: None,
				is_multiplexing: true,
			},
		);

		let err = session_manager
			.resolve_session(&session_id, relay_inputs(vec![server_a], true))
			.expect_err("missing snapshot member should classify as target mismatch");

		assert_eq!(err, ResumeFailureReason::SnapshotMismatch);
	}

	#[test]
	fn resolve_session_rejects_multiplex_snapshot_without_encrypted_session_ids() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let server_a = capture_target("serverA", server_a_capture.path());
		let server_b = capture_target("serverB", server_b_capture.path());
		let encoder = Encoder::base64();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![
				snapshot_member(
					&server_a,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(true, false),
				),
				snapshot_member(
					&server_b,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(false, true),
				),
			],
			MCPSnapshotRouting {
				default_target_name: None,
				is_multiplexing: true,
			},
		);

		let err = session_manager
			.resolve_session(&session_id, relay_inputs(vec![server_a, server_b], true))
			.expect_err("multiplex snapshots should require encrypted session ids");

		assert_eq!(err, ResumeFailureReason::EncryptedSessionIdsRequired);
	}

	#[tokio::test]
	async fn resolve_session_allows_multiplex_snapshot_when_explicitly_opted_in() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let server_a = capture_target("serverA", server_a_capture.path());
		let server_b = capture_target("serverB", server_b_capture.path());
		let encoder = Encoder::base64();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![
				snapshot_member(
					&server_a,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(true, false),
				),
				snapshot_member(
					&server_b,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(false, true),
				),
			],
			MCPSnapshotRouting {
				default_target_name: None,
				is_multiplexing: true,
			},
		);

		let session = session_manager
			.resolve_session(
				&session_id,
				relay_inputs_with_options(vec![server_a, server_b], true, true),
			)
			.expect("explicit opt-in should allow insecure multiplex snapshot resume");

		assert_eq!(session.continuity(), SessionContinuity::Reconstructible);
	}

	#[test]
	fn resolve_session_rejects_single_target_routed_snapshot_without_encrypted_session_ids() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_a = capture_target_with_prefix("serverA", server_a_capture.path(), true);
		let encoder = Encoder::base64();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![snapshot_member(
				&server_a,
				MCPSession {
					session: None,
					backend: None,
				},
				server_info(true, false),
			)],
			MCPSnapshotRouting {
				default_target_name: None,
				is_multiplexing: false,
			},
		);

		let err = session_manager
			.resolve_session(&session_id, relay_inputs(vec![server_a], true))
			.expect_err("single-target routed snapshots should require encrypted session ids");

		assert_eq!(err, ResumeFailureReason::EncryptedSessionIdsRequired);
	}

	#[tokio::test]
	async fn resolve_session_allows_single_target_routed_snapshot_when_explicitly_opted_in() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_a = capture_target_with_prefix("serverA", server_a_capture.path(), true);
		let encoder = Encoder::base64();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![snapshot_member(
				&server_a,
				MCPSession {
					session: None,
					backend: None,
				},
				server_info(true, false),
			)],
			MCPSnapshotRouting {
				default_target_name: None,
				is_multiplexing: false,
			},
		);

		let session = session_manager
			.resolve_session(
				&session_id,
				relay_inputs_with_options(vec![server_a], true, true),
			)
			.expect("explicit opt-in should allow insecure routed snapshot resume");

		assert_eq!(session.continuity(), SessionContinuity::Reconstructible);
	}

	#[tokio::test]
	async fn resume_session_restores_snapshot_membership() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let server_a = capture_target("serverA", server_a_capture.path());
		let server_b = capture_target("serverB", server_b_capture.path());
		let encoder = encrypted_test_encoder();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![snapshot_member(
				&server_b,
				MCPSession {
					session: None,
					backend: None,
				},
				server_info(false, false),
			)],
			MCPSnapshotRouting {
				default_target_name: None,
				is_multiplexing: true,
			},
		);
		let inputs = relay_inputs(vec![server_a, server_b], true);

		let session = session_manager
			.resolve_session(&session_id, inputs)
			.expect("snapshot should resume");

		assert_eq!(session.continuity(), SessionContinuity::Reconstructible);

		assert_eq!(
			session.relay.count(),
			1,
			"resume should restore only snapshot members"
		);
		assert!(
			session.relay.is_multiplexing(),
			"resume should preserve original multiplexing semantics"
		);
		assert!(
			session.relay.parse_resource_name("serverB__tool").is_ok(),
			"degraded multi-target sessions should keep prefixed identifier routing"
		);
		assert!(
			session.relay.parse_resource_name("tool").is_err(),
			"resumed multiplex relay should still reject unprefixed identifiers"
		);

		let result = session
			.relay
			.send_notification(
				test_notification("notifications/test-session-snapshot"),
				IncomingRequestContext::empty(),
			)
			.await;
		assert!(
			result.is_ok(),
			"snapshot notification should route successfully"
		);

		wait_until_contains(
			server_b_capture.path(),
			"\"notifications/test-session-snapshot\"",
		)
		.await;
		assert_not_contains_for(
			server_a_capture.path(),
			"\"notifications/test-session-snapshot\"",
			Duration::from_millis(250),
		)
		.await;
	}

	#[test]
	fn resume_session_returns_none_when_snapshot_target_missing() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let server_a = capture_target("serverA", server_a_capture.path());
		let server_b = capture_target("serverB", server_b_capture.path());
		let encoder = encrypted_test_encoder();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![
				snapshot_member(
					&server_a,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(true, false),
				),
				snapshot_member(
					&server_b,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(false, true),
				),
			],
			MCPSnapshotRouting {
				default_target_name: None,
				is_multiplexing: true,
			},
		);
		let inputs = relay_inputs(vec![server_a], true);

		let err = session_manager
			.resolve_session(&session_id, inputs)
			.expect_err("missing snapshot member should classify as target mismatch");

		assert_eq!(err, ResumeFailureReason::SnapshotMismatch);
	}

	#[test]
	fn resume_session_rejects_legacy_snapshot() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let encoder = encrypted_test_encoder();
		let session_manager = SessionManager::new(encoder.clone());
		let legacy_session_id = encoder
			.encrypt(r#"{"t":"mcp","s":[{"s":"upstream-session","b":"127.0.0.1:8080"}]}"#)
			.expect("legacy state should encode");
		let inputs = relay_inputs(
			vec![capture_target("serverA", server_a_capture.path())],
			true,
		);

		let err = session_manager
			.resolve_session(&legacy_session_id, inputs)
			.expect_err("legacy positional snapshots should classify as malformed");

		assert_eq!(err, ResumeFailureReason::MalformedHandle);
	}

	#[tokio::test]
	async fn resume_session_preserves_allow_degraded() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_a = capture_target("serverA", server_a_capture.path());
		let encoder = encrypted_test_encoder();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![snapshot_member(
				&server_a,
				MCPSession {
					session: None,
					backend: None,
				},
				server_info(false, false),
			)],
			MCPSnapshotRouting {
				default_target_name: Some("serverA".to_string()),
				is_multiplexing: false,
			},
		);
		let inputs = relay_inputs(vec![server_a], true);
		let session = session_manager
			.resolve_session(&session_id, inputs)
			.expect("snapshot should resume");
		let ctx = IncomingRequestContext::empty();

		let deleted = session.relay.send_fanout_deletion(ctx.clone()).await;
		assert!(deleted.is_ok(), "deletion should succeed");

		let result = session
			.relay
			.send_notification(test_notification("notifications/test-after-delete"), ctx)
			.await;
		assert!(
			result.is_ok(),
			"resume should keep runtime degraded handling after strict restoration"
		);
	}

	#[tokio::test]
	async fn resume_session_restores_capability_routing() {
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let server_a = capture_target("serverA", server_a_capture.path());
		let server_b = capture_target("serverB", server_b_capture.path());
		let encoder = encrypted_test_encoder();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![
				snapshot_member(
					&server_a,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(true, false),
				),
				snapshot_member(
					&server_b,
					MCPSession {
						session: None,
						backend: None,
					},
					server_info(false, true),
				),
			],
			MCPSnapshotRouting {
				default_target_name: None,
				is_multiplexing: true,
			},
		);
		let session = session_manager
			.resolve_session(&session_id, relay_inputs(vec![server_a, server_b], true))
			.expect("snapshot should resume");

		assert_eq!(
			session.relay.upstreams_with_tools(),
			vec![Strng::from("serverA")],
			"resumed relay should preserve initialize-time tool capabilities"
		);
		assert_eq!(
			session.relay.upstreams_with_prompts(),
			vec![Strng::from("serverB")],
			"resumed relay should preserve initialize-time prompt capabilities"
		);
	}

	#[test]
	fn resume_session_returns_none_when_target_fingerprint_changes() {
		let original_capture = NamedTempFile::new().expect("temp file");
		let drifted_capture = NamedTempFile::new().expect("temp file");
		let original_target = capture_target("serverA", original_capture.path());
		let drifted_target = capture_target("serverA", drifted_capture.path());
		let encoder = Encoder::base64();
		let session_manager = SessionManager::new(encoder.clone());
		let session_id = encode_snapshot(
			&encoder,
			vec![snapshot_member(
				&original_target,
				MCPSession {
					session: None,
					backend: None,
				},
				server_info(true, false),
			)],
			MCPSnapshotRouting {
				default_target_name: Some("serverA".to_string()),
				is_multiplexing: false,
			},
		);

		let err = session_manager
			.resolve_session(&session_id, relay_inputs(vec![drifted_target], true))
			.expect_err("drifted same-named target should classify as target mismatch");

		assert_eq!(err, ResumeFailureReason::SnapshotMismatch);
	}
}
