use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use ::http::StatusCode;
use ::http::request::Parts;
use agent_core::metrics::Recorder;
use rmcp::ErrorData;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, ErrorCode, JsonRpcError, RequestId};
use rmcp::transport::common::http_header::EVENT_STREAM_MIME_TYPE;
use rmcp::transport::common::server_side_http::{ServerSseMessage, session_id};
use sse_stream::{KeepAlive, Sse, SseBody};

use crate::http::Response;
use crate::mcp::handler::Relay;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, rbac};
use crate::{mcp, *};

#[derive(Debug, Clone)]
pub struct Session {
	relay: Arc<Relay>,
	pub id: Arc<str>,
}

impl Session {
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

	pub async fn delete_session(&self, parts: Parts) -> Response {
		let ctx = IncomingRequestContext::new(parts);
		self
			.relay
			.send_fanout_deletion(ctx)
			.await
			.unwrap_or_else(Self::handle_error(None))
	}

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
		match message {
			ClientJsonRpcMessage::Request(mut r) => {
				let method = r.request.method();
				let (_span, _, log, cel) = mcp::handler::setup_request_log2(&parts, method);

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
	pub fn create_session(&self, relay: Relay) -> Session {
		let id = session_id();
		let sess = Session {
			id: id.clone(),
			relay: Arc::new(relay),
		};
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(id.to_string(), sess.clone());
		sess
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
