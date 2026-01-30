use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use ::http::StatusCode;
use ::http::header::CONTENT_TYPE;
use ::http::request::Parts;
use agent_core::version::BuildInfo;
use anyhow::anyhow;
use futures_util::StreamExt;
use headers::HeaderMapExt;
use rmcp::model::{
	ClientInfo, ClientJsonRpcMessage, ClientNotification, ClientRequest, ConstString, Implementation,
	ProtocolVersion, RequestId, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE};
use sse_stream::{KeepAlive, Sse, SseBody, SseStream};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::http::Response;
use crate::mcp::handler::{Relay, RelayInputs};
use crate::mcp::mergestream::Messages;
use crate::mcp::streamablehttp::{ServerSseMessage, StreamableHttpPostResponse};
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, MCPOperation, rbac};
use crate::proxy::ProxyError;
use crate::{mcp, *};

#[derive(Debug, Clone)]
pub struct Session {
	encoder: http::sessionpersistence::Encoder,
	relay: Arc<Relay>,
	pub id: Arc<str>,
	tx: Option<Sender<ServerJsonRpcMessage>>,
}

impl Session {
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
		Self::handle_error(req_id, self.send_internal(parts, message).await).await
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
		let is_init = matches!(&message, ClientJsonRpcMessage::Request(r) if matches!(&r.request, &ClientRequest::InitializeRequest(_)));
		if !is_init {
			// first, send the initialize
			let init_request = rmcp::model::InitializeRequest {
				method: Default::default(),
				params: get_client_info(),
				extensions: Default::default(),
			};
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
		let ctx = IncomingRequestContext::new(&parts);
		let (_span, log, _cel) = mcp::handler::setup_request_log(parts, "delete_session");
		let session_id = self.id.to_string();
		log.non_atomic_mutate(|l| {
			// NOTE: l.method_name keep None to respect the metrics logic: not handle GET, DELETE.
			l.session_id = Some(session_id);
		});
		Self::handle_error(None, self.relay.send_fanout_deletion(ctx).await).await
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
		let ctx = IncomingRequestContext::new(&parts);
		let (_span, log, _cel) = mcp::handler::setup_request_log(parts, "get_stream");
		let session_id = self.id.to_string();
		log.non_atomic_mutate(|l| {
			// NOTE: l.method_name keep None to respect the metrics logic: which do not want to handle GET, DELETE.
			l.session_id = Some(session_id);
		});
		Self::handle_error(None, self.relay.send_fanout_get(ctx).await).await
	}

	async fn handle_error(
		req_id: Option<RequestId>,
		d: Result<Response, UpstreamError>,
	) -> Result<Response, ProxyError> {
		match d {
			Ok(r) => Ok(r),
			Err(UpstreamError::Http(ClientError::Status(resp))) => {
				let resp = http::SendDirectResponse::new(*resp)
					.await
					.map_err(ProxyError::Body)?;
				Err(mcp::Error::UpstreamError(Box::new(resp)).into())
			},
			Err(UpstreamError::Proxy(p)) => Err(p),
			Err(UpstreamError::Authorization {
				resource_type,
				resource_name,
			}) if req_id.is_some() => {
				Err(mcp::Error::Authorization(req_id.unwrap(), resource_type, resource_name).into())
			},
			// TODO: this is too broad. We have a big tangle of errors to untangle though
			Err(e) => Err(mcp::Error::SendError(req_id, e.to_string()).into()),
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
				let method = r.request.method();
				let ctx = IncomingRequestContext::new(&parts);
				let (_span, log, cel) = mcp::handler::setup_request_log(parts, method);
				let session_id = self.id.to_string();
				log.non_atomic_mutate(|l| {
					l.method_name = Some(method.to_string());
					l.session_id = Some(session_id);
				});
				match &mut r.request {
					ClientRequest::InitializeRequest(ir) => {
						// Currently, we cannot support roots until we have a mapping of downstream and upstream ID.
						// However, the clients can tell the server they support roots.
						// Instead, we hijack this to tell them not to so they do not send requests that we cannot
						// actually support
						// This could probably be more easily done without multiplexing but for now neither supports.
						ir.params.capabilities.roots = None;

						let pv = ir.params.protocol_version.clone();
						let res = self
							.relay
							.send_fanout(
								r,
								ctx,
								self
									.relay
									.merge_initialize(pv, self.relay.is_multiplexing()),
							)
							.await;
						if let Some(sessions) = self.relay.get_sessions() {
							let s = http::sessionpersistence::SessionState::MCP(
								http::sessionpersistence::MCPSessionState { sessions },
							);
							if let Ok(id) = s.encode(&self.encoder) {
								self.id = id.into();
							}
						}
						res
					},
					ClientRequest::ListToolsRequest(_) => {
						log.non_atomic_mutate(|l| {
							l.resource = Some(MCPOperation::Tool);
						});
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_tools(cel))
							.await
					},
					ClientRequest::PingRequest(_) | ClientRequest::SetLevelRequest(_) => {
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_empty())
							.await
					},
					ClientRequest::ListPromptsRequest(_) => {
						log.non_atomic_mutate(|l| {
							l.resource = Some(MCPOperation::Prompt);
						});
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_prompts(cel))
							.await
					},
					ClientRequest::ListResourcesRequest(_) => {
						if !self.relay.is_multiplexing() {
							log.non_atomic_mutate(|l| {
								l.resource = Some(MCPOperation::Resource);
							});
							self
								.relay
								.send_fanout(r, ctx, self.relay.merge_resources(cel))
								.await
						} else {
							// TODO(https://github.com/agentgateway/agentgateway/issues/404)
							// Find a mapping of URL
							Err(UpstreamError::InvalidMethodWithMultiplexing(
								r.request.method().to_string(),
							))
						}
					},
					ClientRequest::ListResourceTemplatesRequest(_) => {
						if !self.relay.is_multiplexing() {
							log.non_atomic_mutate(|l| {
								l.resource = Some(MCPOperation::ResourceTemplates);
							});
							self
								.relay
								.send_fanout(r, ctx, self.relay.merge_resource_templates(cel))
								.await
						} else {
							// TODO(https://github.com/agentgateway/agentgateway/issues/404)
							// Find a mapping of URL
							Err(UpstreamError::InvalidMethodWithMultiplexing(
								r.request.method().to_string(),
							))
						}
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
							&rbac::ResourceType::Tool(rbac::ResourceId::new(
								service_name.to_string(),
								tool.to_string(),
							)),
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
							&rbac::ResourceType::Prompt(rbac::ResourceId::new(
								service_name.to_string(),
								prompt.to_string(),
							)),
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
						if let Some(service_name) = self.relay.default_target_name() {
							let uri = rrr.params.uri.clone();
							log.non_atomic_mutate(|l| {
								l.target_name = Some(service_name.to_string());
								l.resource_name = Some(uri.to_string());
								l.resource = Some(MCPOperation::Resource);
							});
							if !self.relay.policies.validate(
								&rbac::ResourceType::Resource(rbac::ResourceId::new(
									service_name.to_string(),
									uri.to_string(),
								)),
								&cel,
							) {
								return Err(UpstreamError::Authorization {
									resource_type: "resource".to_string(),
									resource_name: uri.to_string(),
								});
							}
							self.relay.send_single_without_multiplexing(r, ctx).await
						} else {
							// TODO(https://github.com/agentgateway/agentgateway/issues/404)
							// Find a mapping of URL
							Err(UpstreamError::InvalidMethodWithMultiplexing(
								r.request.method().to_string(),
							))
						}
					},

					ClientRequest::ListTasksRequest(_)
					| ClientRequest::GetTaskInfoRequest(_)
					| ClientRequest::GetTaskResultRequest(_)
					| ClientRequest::CancelTaskRequest(_)
					| ClientRequest::SubscribeRequest(_)
					| ClientRequest::UnsubscribeRequest(_)
					| ClientRequest::CustomRequest(_) => {
						// TODO(https://github.com/agentgateway/agentgateway/issues/404)
						Err(UpstreamError::InvalidMethod(r.request.method().to_string()))
					},
					ClientRequest::CompleteRequest(_) => {
						// For now, we don't have a sane mapping of incoming requests to a specific
						// downstream service when multiplexing. Only forward when we have only one backend.
						self.relay.send_single_without_multiplexing(r, ctx).await
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
				let (_span, log, _cel) = mcp::handler::setup_request_log(parts, method);
				let session_id = self.id.to_string();
				log.non_atomic_mutate(|l| {
					l.method_name = Some(method.to_string());
					l.session_id = Some(session_id);
				});
				// TODO: the notification needs to be fanned out in some cases and sent to a single one in others
				// however, we don't have a way to map to the correct service yet
				self.relay.send_notification(r, ctx).await
			},

			_ => Err(UpstreamError::InvalidRequest(
				"unsupported message type".to_string(),
			)),
		}
	}
}

#[derive(Debug)]
pub struct SessionManager {
	encoder: http::sessionpersistence::Encoder,
	sessions: RwLock<HashMap<String, Session>>,
}

fn session_id() -> Arc<str> {
	uuid::Uuid::new_v4().to_string().into()
}

impl SessionManager {
	pub fn new(encoder: http::sessionpersistence::Encoder) -> Self {
		Self {
			encoder,
			sessions: Default::default(),
		}
	}

	pub fn get_session(&self, id: &str, builder: RelayInputs) -> Option<Session> {
		Some(
			self
				.sessions
				.read()
				.ok()?
				.get(id)
				.cloned()?
				.with_inputs(builder),
		)
	}

	pub fn get_or_resume_session(
		&self,
		id: &str,
		builder: RelayInputs,
	) -> Result<Option<Session>, mcp::Error> {
		if let Some(s) = self.sessions.read().expect("poisoned").get(id).cloned() {
			return Ok(Some(s.with_inputs(builder)));
		}
		let d = http::sessionpersistence::SessionState::decode(id, &self.encoder)
			.map_err(|_| mcp::Error::InvalidSessionIdHeader)?;
		let http::sessionpersistence::SessionState::MCP(state) = d else {
			return Ok(None);
		};
		let relay = builder.build_new_connections()?;
		let n = relay.count();
		if state.sessions.len() != n {
			warn!(
				"failed to resume session: sessions {} did not match config {}",
				state.sessions.len(),
				n
			);
			return Ok(None);
		}
		relay.set_sessions(state.sessions);

		let sess = Session {
			id: id.into(),
			relay: Arc::new(relay),
			tx: None,
			encoder: self.encoder.clone(),
		};
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(id.to_string(), sess.clone());
		Ok(Some(sess))
	}

	/// create_session establishes an MCP session.
	pub fn create_session(&self, relay: Relay) -> Session {
		let id = session_id();

		// Do NOT insert yet
		Session {
			id: id.clone(),
			relay: Arc::new(relay),
			tx: None,
			encoder: self.encoder.clone(),
		}
	}

	pub fn insert_session(&self, sess: Session) {
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(sess.id.to_string(), sess);
	}

	/// create_stateless_session creates a session for stateless mode.
	/// Unlike create_session, this does NOT register the session in the session manager.
	/// The caller is responsible for calling session.delete_session() when done
	/// to clean up upstream resources (e.g., stdio processes).
	pub fn create_stateless_session(&self, relay: Relay) -> Session {
		let id = session_id();
		Session {
			id,
			relay: Arc::new(relay),
			tx: None,
			encoder: self.encoder.clone(),
		}
	}

	/// create_legacy_session establishes a legacy SSE session.
	/// These will have the ability to send messages to them via a channel.
	pub fn create_legacy_session(&self, relay: Relay) -> (Session, Receiver<ServerJsonRpcMessage>) {
		let (tx, rx) = tokio::sync::mpsc::channel(64);
		let id = session_id();
		let sess = Session {
			id: id.clone(),
			relay: Arc::new(relay),
			tx: Some(tx),
			encoder: self.encoder.clone(),
		};
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(id.to_string(), sess.clone());
		(sess, rx)
	}

	pub async fn delete_session(&self, id: &str, parts: Parts) -> Option<Response> {
		let sess = {
			let mut sm = self.sessions.write().expect("write lock");
			sm.remove(id)?
		};
		// Swallow the error
		sess.delete_session(parts).await.ok()
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

impl Drop for SessionDropper {
	fn drop(&mut self) {
		let Some((s, parts)) = self.s.take() else {
			return;
		};
		let mut sm = self.sm.sessions.write().expect("write lock");
		debug!("delete session {}", s.id);
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

fn get_client_info() -> ClientInfo {
	ClientInfo {
		meta: None,
		protocol_version: ProtocolVersion::V_2025_06_18,
		capabilities: rmcp::model::ClientCapabilities {
			experimental: None,
			roots: None,
			sampling: None,
			elicitation: None,
			tasks: None,
			extensions: None,
		},
		client_info: Implementation {
			name: "agentgateway".to_string(),
			version: BuildInfo::new().version.to_string(),
			..Default::default()
		},
	}
}
