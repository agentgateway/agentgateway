use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use ::http::StatusCode;
use ::http::header::CONTENT_TYPE;
use ::http::request::Parts;
use agent_core::metrics::Recorder;
use agent_core::version::BuildInfo;
use anyhow::anyhow;
use futures_util::StreamExt;
use rmcp::ErrorData;
use rmcp::model::{
	ClientInfo, ClientJsonRpcMessage, ClientRequest, ErrorCode, Implementation, JsonRpcError,
	ProtocolVersion, RequestId, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE};
use rmcp::transport::common::server_side_http::{ServerSseMessage, session_id};
use rmcp::transport::streamable_http_client::StreamableHttpPostResponse;
use sse_stream::{KeepAlive, Sse, SseBody, SseStream};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::http::Response;
use crate::mcp::handler::Relay;
use crate::mcp::mergestream::Messages;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, rbac};
use crate::{mcp, *};

#[derive(Debug, Clone)]
pub struct Session {
	relay: Arc<Relay>,
	pub id: Arc<str>,
	tx: Option<Sender<ServerJsonRpcMessage>>,
}

impl Session {
	/// send a message to upstream server(s)
	pub async fn send(&self, parts: Parts, message: ClientJsonRpcMessage) -> Response {
		let req_id = match &message {
			ClientJsonRpcMessage::Request(r) => Some(r.id.clone()),
			_ => None,
		};
		self
			.send_internal(parts, message)
			.await
			.unwrap_or_else(Self::handle_error(req_id))
	}
	/// send a message to upstream server(s), when using stateless mode. In stateless mode, every message
	/// is wrapped in an InitializeRequest (except the actual InitializeRequest from the downstream).
	/// This ensures servers that require an InitializeRequest behave correctly.
	/// In the future, we may have a mode where we know the downstream is stateless as well, and can just forward as-is.
	pub async fn stateless_send_and_initialize(
		&self,
		parts: Parts,
		message: ClientJsonRpcMessage,
	) -> Response {
		let is_init = matches!(&message, ClientJsonRpcMessage::Request(r) if matches!(&r.request, &ClientRequest::InitializeRequest(_)));
		if !is_init {
			// first, send the initialize
			let init_request = rmcp::model::InitializeRequest {
				method: Default::default(),
				params: get_client_info(),
				extensions: Default::default(),
			};
			let res = self
				.send(
					parts.clone(),
					ClientJsonRpcMessage::request(init_request.into(), RequestId::Number(0)),
				)
				.await;
			if !res.status().is_success() {
				return res;
			}

			// And we need to notify as well.
			let notification = ClientJsonRpcMessage::notification(
				rmcp::model::InitializedNotification {
					method: Default::default(),
					extensions: Default::default(),
				}
				.into(),
			);
			let res = self.send(parts.clone(), notification).await;
			if !res.status().is_success() {
				return res;
			}
		}
		// Now we can send the message like normal
		self.send(parts, message).await
	}

	/// delete any active sessions
	pub async fn delete_session(&self, parts: Parts) -> Response {
		let ctx = IncomingRequestContext::new(parts);
		self
			.relay
			.send_fanout_deletion(ctx)
			.await
			.unwrap_or_else(Self::handle_error(None))
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
				let event_stream = SseStream::from_byte_stream(resp.into_body().into_data_stream()).boxed();
				Ok(StreamableHttpPostResponse::Sse(event_stream, None))
			},
			Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
				trace!("forward SSE got single JSON response");
				let message = json::from_response_body::<ServerJsonRpcMessage>(resp)
					.await
					.map_err(ClientError::new)?;
				Ok(StreamableHttpPostResponse::Json(message, None))
			},
			_ => {
				trace!("forward SSE got accepted, no action needed");
				return Ok(());
			},
		}?;
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
	pub async fn get_stream(&self, parts: Parts) -> Response {
		let ctx = IncomingRequestContext::new(parts);
		self
			.relay
			.send_fanout_get(ctx)
			.await
			.unwrap_or_else(Self::handle_error(None))
	}

	fn handle_error(req_id: Option<RequestId>) -> impl FnOnce(UpstreamError) -> Response {
		move |e| {
			if let UpstreamError::Http(ClientError::Status(resp)) = e {
				// Forward response as-is
				return *resp;
			}
			let err = if let Some(req_id) = req_id {
				serde_json::to_string(&JsonRpcError {
					jsonrpc: Default::default(),
					id: req_id,
					error: ErrorData {
						code: ErrorCode::INTERNAL_ERROR,
						message: format!("failed to send message: {e}",).into(),
						data: None,
					},
				})
				.ok()
			} else {
				None
			};
			http_error(
				StatusCode::INTERNAL_SERVER_ERROR,
				err.unwrap_or_else(|| format!("failed to send message: {e}")),
			)
		}
	}

	async fn send_internal(
		&self,
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
				let (_span, log, cel) = mcp::handler::setup_request_log(&parts, method);

				let ctx = IncomingRequestContext::new(parts);
				match &mut r.request {
					ClientRequest::InitializeRequest(_) => {
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_initialize())
							.await
					},
					ClientRequest::ListToolsRequest(_) => {
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_tools(cel.clone()))
							.await
					},
					ClientRequest::PingRequest(_) | ClientRequest::SetLevelRequest(_) => {
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_empty())
							.await
					},
					ClientRequest::ListPromptsRequest(_) => {
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_prompts(cel.clone()))
							.await
					},
					ClientRequest::ListResourcesRequest(_) => {
						if !self.relay.is_multiplexing() {
							self
								.relay
								.send_fanout(r, ctx, self.relay.merge_resources(cel.clone()))
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
							self
								.relay
								.send_fanout(r, ctx, self.relay.merge_resource_templates(cel.clone()))
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
							l.tool_call_name = Some(tool.to_string());
							l.target_name = Some(service_name.to_string());
						});
						if !self.relay.policies.validate(
							&rbac::ResourceType::Tool(rbac::ResourceId::new(
								service_name.to_string(),
								tool.to_string(),
							)),
							cel.as_ref(),
						) {
							return Err(UpstreamError::Authorization);
						}

						self.relay.metrics.record(
							crate::mcp::metrics::ToolCall {
								server: service_name.to_string(),
								name: tool.to_string(),
								params: vec![],
							},
							(),
						);
						let tn = tool.to_string();
						ctr.params.name = tn.into();
						self.relay.send_single(r, ctx, service_name).await
					},
					ClientRequest::GetPromptRequest(gpr) => {
						let name = gpr.params.name.clone();
						let (service_name, prompt) = self.relay.parse_resource_name(&name)?;
						log.non_atomic_mutate(|l| {
							l.target_name = Some(service_name.to_string());
						});
						if !self.relay.policies.validate(
							&rbac::ResourceType::Prompt(rbac::ResourceId::new(
								service_name.to_string(),
								prompt.to_string(),
							)),
							cel.as_ref(),
						) {
							return Err(UpstreamError::Authorization);
						}
						gpr.params.name = prompt.to_string();
						self.relay.send_single(r, ctx, service_name).await
					},
					ClientRequest::ReadResourceRequest(rrr) => {
						if let Some(service_name) = self.relay.default_target_name() {
							let uri = rrr.params.uri.clone();
							log.non_atomic_mutate(|l| {
								l.target_name = Some(service_name.to_string());
							});
							if !self.relay.policies.validate(
								&rbac::ResourceType::Resource(rbac::ResourceId::new(
									service_name.to_string(),
									uri.to_string(),
								)),
								cel.as_ref(),
							) {
								return Err(UpstreamError::Authorization);
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
					ClientRequest::SubscribeRequest(_) | ClientRequest::UnsubscribeRequest(_) => {
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
				let ctx = IncomingRequestContext::new(parts);
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

#[derive(Default, Debug)]
pub struct SessionManager {
	sessions: RwLock<HashMap<String, Session>>,
}

impl SessionManager {
	pub fn get_session(&self, id: &str) -> Option<Session> {
		self.sessions.read().ok()?.get(id).cloned()
	}

	/// create_session establishes an MCP session.
	pub fn create_session(&self, relay: Relay) -> Session {
		let id = session_id();
		let sess = Session {
			id: id.clone(),
			relay: Arc::new(relay),
			tx: None,
		};
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(id.to_string(), sess.clone());
		sess
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
		Some(sess.delete_session(parts).await)
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

fn http_error(status: StatusCode, body: impl Into<http::Body>) -> Response {
	::http::Response::builder()
		.status(status)
		.body(body.into())
		.expect("valid response")
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
		protocol_version: ProtocolVersion::V_2025_06_18,
		capabilities: rmcp::model::ClientCapabilities {
			experimental: None,
			roots: None,
			sampling: None,
			elicitation: None,
		},
		client_info: Implementation {
			name: "agentgateway".to_string(),
			version: BuildInfo::new().version.to_string(),
			..Default::default()
		},
	}
}
