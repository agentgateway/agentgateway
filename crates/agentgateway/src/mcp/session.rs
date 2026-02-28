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
use rmcp::ErrorData;
use rmcp::model::{
	ClientInfo, ClientJsonRpcMessage, ClientNotification, ClientRequest, ConstString, ErrorCode,
	Implementation, JsonRpcError, JsonRpcRequest, ProtocolVersion, RequestId, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE};
use sse_stream::{KeepAlive, Sse, SseBody, SseStream};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::http::Response;
use crate::mcp::handler::{Relay, RelayInputs};
use crate::mcp::mergestream::{MergeFn, Messages};
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
				let (_span, log, cel) = mcp::handler::setup_request_log(parts, &method);
				let session_id = self.id.to_string();
				log.non_atomic_mutate(|l| {
					l.method_name = Some(method.clone());
					l.session_id = Some(session_id);
				});
				match &mut r.request {
					ClientRequest::InitializeRequest(ir) => {
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
								http::sessionpersistence::MCPSessionState::new(sessions),
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
				let (_span, log, _cel) = mcp::handler::setup_request_log(parts, method);
				let session_id = self.id.to_string();
				log.non_atomic_mutate(|l| {
					l.method_name = Some(method.to_string());
					l.session_id = Some(session_id);
				});
				// Relay handles targeted routing for notifications with encoded upstream identifiers
				// (for example cancellation/request ids and progress tokens), and fans out the rest.
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::mcp::upstream::UpstreamError;

	#[tokio::test]
	async fn handle_error_invalid_method_returns_jsonrpc() {
		let resp = Session::handle_error(
			Some(RequestId::Number(7)),
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
	async fn handle_error_http_passthrough_preserves_status() {
		let resp = ::http::Response::builder()
			.status(StatusCode::BAD_GATEWAY)
			.body(crate::http::Body::from("nope"))
			.expect("response");
		let err = Session::handle_error(
			Some(RequestId::Number(1)),
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
}
