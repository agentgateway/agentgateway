use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use ::http::StatusCode;
use ::http::header::CONTENT_TYPE;
use ::http::request::Parts;
use agent_core::prelude::AssertSize;
use agent_core::version::BuildInfo;
use anyhow::anyhow;
use futures_util::StreamExt;
use headers::HeaderMapExt;
use rmcp::model::{
	ClientInfo, ClientJsonRpcMessage, ClientNotification, ClientRequest, ConstString, GetMeta,
	Implementation, InitializeRequest, JsonRpcRequest, ProtocolVersion, Reference, RequestId,
	ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE};
use sse_stream::{KeepAlive, Sse, SseBody, SseStream};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::http::Response;
use crate::mcp::handler::{Relay, RelayInputs, ResolveKind};
use crate::mcp::mergestream::Messages;
use crate::mcp::streamablehttp::{ServerSseMessage, StreamableHttpPostResponse};
use crate::mcp::subscriptions::ResourceSubscription;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, rbac};
use crate::proxy::ProxyError;
use crate::telemetry::log::{AsyncLog, SpanWriteOnDrop};
use crate::{mcp, *};

#[derive(Debug, Clone)]
pub struct Session {
	encoder: http::sessionpersistence::Encoder,
	relay: Arc<Relay>,
	pub id: Arc<str>,
	tx: Option<Sender<ServerJsonRpcMessage>>,
}

#[derive(Debug, Clone)]
struct SessionEntry {
	session: Session,
	last_access: Instant,
	idle_ttl: Duration,
}

const SESSION_REAP_INTERVAL: Duration = Duration::from_secs(30);

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
		let res = self
			.send_internal(parts, message)
			.assert_size::<{ 6 * 1024 }>()
			.await;
		Self::handle_error(req_id, res, false).await
	}

	/// Send a downstream message to upstream server(s) in gateway stateless mode.
	/// When `initialize_upstream` is true, every non-initialize message gets a
	/// gateway-generated InitializeRequest first because many legacy servers
	/// require initialize before any other request. The caller sets it to false
	/// for modern requests, which are forwarded as-is without a synthetic
	/// handshake.
	pub async fn stateless_send_and_initialize(
		&mut self,
		parts: Parts,
		message: ClientJsonRpcMessage,
		initialize_upstream: bool,
	) -> Result<Response, ProxyError> {
		let req_id = match &message {
			ClientJsonRpcMessage::Request(r) => Some(r.id.clone()),
			_ => None,
		};
		let is_init = matches!(&message,
			ClientJsonRpcMessage::Request(r) if matches!(r.request, ClientRequest::InitializeRequest(_)));
		if initialize_upstream && !is_init {
			let mut client_info = get_client_info();
			if let Some(protocol_version) =
				crate::mcp::streamablehttp::protocol_version_header(&parts.headers, req_id.clone(), true)?
			{
				client_info.protocol_version = protocol_version;
			}
			let init_request = rmcp::model::InitializeRequest::new(client_info);
			let request_type = match &message {
				ClientJsonRpcMessage::Request(r) => Some(&r.request),
				_ => None,
			};
			match request_type {
				// Initialize only the target named by a prefixed call. With prefixMode: never,
				// the list request used to find that target requires every target to be initialized.
				Some(ClientRequest::CallToolRequest(_)) | Some(ClientRequest::GetPromptRequest(_))
					if !self.relay.needs_resolution() =>
				{
					let name = match request_type {
						Some(ClientRequest::CallToolRequest(ctr)) => ctr.params.name.to_string(),
						Some(ClientRequest::GetPromptRequest(gpr)) => gpr.params.name.clone(),
						_ => unreachable!("match arm guarantees single-target request type"),
					};
					let (service_name, _) = match self.relay.parse_resource_name(&name) {
						Ok(target) => target,
						Err(err) => return Self::handle_error(req_id.clone(), Err(err), false).await,
					};
					let res = self
						.send_init_single(parts.clone(), init_request, service_name)
						.await;
					if let Some(sessions) = self.relay.get_sessions() {
						let s = http::sessionpersistence::SessionState::MCP(
							http::sessionpersistence::MCPSessionState::new(sessions),
						);
						if let Ok(id) = s.encode(&self.encoder) {
							self.id = id.into();
						}
					}
					Self::handle_error(Some(RequestId::Number(0)), res, false).await?;
					// Now send the initialized notification
					let _ = Self::handle_error(
						None,
						self
							.send_initialized_notification_single(parts.clone(), service_name)
							.await,
						false,
					)
					.await?;
				},
				_ => {
					// We should fan out the initialize request to all MCP servers
					let _ = self
						.send(
							parts.clone(),
							ClientJsonRpcMessage::request(init_request.into(), RequestId::Number(0)),
						)
						.await?;
					let notification = ClientJsonRpcMessage::notification(
						rmcp::model::InitializedNotification {
							method: Default::default(),
							extensions: Default::default(),
						}
						.into(),
					);
					let _ = self.send(parts.clone(), notification).await?;
				},
			}
		}
		// Now we can send the message like normal (if it's tools/call, it'll go to the initialized target)
		if initialize_upstream {
			return self.send(parts, message).await;
		}
		let res = self
			.send_internal(parts, message)
			.assert_size::<{ 6 * 1024 }>()
			.await;
		match res {
			// Modern requests are never part of a legacy session, so method-not-found can use its
			// 404 status; on the legacy `send` path a 404 would signal session termination.
			Err(UpstreamError::InvalidMethod(method)) if req_id.is_some() => {
				Err(mcp::Error::MethodNotFound(req_id, method).into())
			},
			other => Self::handle_error(req_id, other, true).await,
		}
	}

	pub fn with_inputs(mut self, inputs: RelayInputs) -> Self {
		self.relay = Arc::new(self.relay.with_policies(inputs.policies));
		self
	}

	async fn authorize_prompt_request<'a, 'b: 'a>(
		&'a self,
		name: &'b str,
		method: &str,
		span: &mut SpanWriteOnDrop,
		log: &AsyncLog<mcp::MCPInfo>,
		cel: &rbac::CelExecWrapper,
		ctx: &IncomingRequestContext,
	) -> Result<(Cow<'a, str>, &'b str), UpstreamError> {
		let (service_name, prompt) = self
			.relay
			.resolve_resource_name(ResolveKind::Prompt, name, ctx)
			.await?;
		span.rename_span(format!("{method} {service_name}"));
		log.non_atomic_mutate(|l| {
			l.set_prompt(service_name.to_string(), prompt.to_string());
		});
		if !self.relay.policies.validate(
			&rbac::ResourceType::Prompt(rbac::ResourceId::new(
				service_name.to_string(),
				prompt.to_string(),
			)),
			cel,
		) {
			return Err(UpstreamError::Authorization {
				resource_type: "prompt".to_string(),
				resource_name: name.to_string(),
			});
		}
		Ok((service_name, prompt))
	}

	fn authorize_resource_request(
		&self,
		service_name: &str,
		uri: &str,
		method: &str,
		span: &mut SpanWriteOnDrop,
		log: &AsyncLog<mcp::MCPInfo>,
		cel: &rbac::CelExecWrapper,
	) -> Result<(), UpstreamError> {
		span.rename_span(format!("{method} {service_name}"));
		log.non_atomic_mutate(|l| {
			l.set_resource(service_name.to_string(), uri.to_string());
		});
		if !self.relay.policies.validate(
			&rbac::ResourceType::Resource(rbac::ResourceId::new(
				service_name.to_string(),
				uri.to_string(),
			)),
			cel,
		) {
			return Err(UpstreamError::Authorization {
				resource_type: "resource".to_string(),
				resource_name: uri.to_string(),
			});
		}
		Ok(())
	}

	#[allow(clippy::too_many_arguments)]
	async fn authorize_with_ctx<P>(
		&self,
		backend: &str,
		method: &str,
		params: &mut P,
		ctx: &mut IncomingRequestContext,
		res: rbac::ResourceType,
		resource_type: &str,
		resource_name: &str,
	) -> Result<(), UpstreamError>
	where
		P: serde::Serialize + serde::de::DeserializeOwned,
	{
		// run guardrails before other policies, as it may add context to CEL
		self
			.relay
			.maybe_run_guardrails_call_request(backend, method, params, ctx)
			.await?;
		let cel = rbac::CelExecWrapper::new(ctx.as_request().map(|_| ()));
		if self.relay.policies.validate(&res, &cel) {
			Ok(())
		} else {
			Err(UpstreamError::Authorization {
				resource_type: resource_type.to_string(),
				resource_name: resource_name.to_string(),
			})
		}
	}

	/// True when some upstream's `delete` does teardown work even without an upstream
	/// session id (stdio processes, SSE streams).
	pub fn has_connection_teardown(&self) -> bool {
		self.relay.upstreams.has_connection_teardown()
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
		Self::handle_error(None, self.relay.send_fanout_deletion(ctx).await, false).await
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
				let event_stream = SseStream::from_bytes_stream(body.into_data_stream()).boxed();
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
		Self::handle_error(None, self.relay.send_fanout_get(ctx).await, false).await
	}

	async fn handle_error(
		req_id: Option<RequestId>,
		d: Result<Response, UpstreamError>,
		downstream_modern: bool,
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
			Err(UpstreamError::McpGuardrails(rej)) if req_id.is_some() => {
				Err(mcp::Error::McpGuardrails(req_id.unwrap(), rej).into())
			},
			Err(UpstreamError::InvalidRequest(message)) if req_id.is_some() && downstream_modern => {
				Err(mcp::Error::InvalidParams(req_id, message).into())
			},
			Err(UpstreamError::Unavailable(message)) if req_id.is_some() && downstream_modern => {
				Err(mcp::Error::Unavailable(req_id, message).into())
			},
			// TODO: this is too broad. We have a big tangle of errors to untangle though
			Err(e) => Err(mcp::Error::SendError(req_id, e.to_string()).into()),
		}
	}

	async fn send_init_single(
		&self,
		parts: Parts,
		mut init_request: InitializeRequest,
		service_name: &str,
	) -> Result<Response, UpstreamError> {
		let method = init_request.method.as_str().to_string();
		let ctx = IncomingRequestContext::new(&parts);
		let (_, log, _) = mcp::handler::setup_request_log(parts, &method);
		let session_id = self.id.to_string();
		log.non_atomic_mutate(|l| {
			l.method_name = Some(method.clone());
			l.session_id = Some(session_id);
		});

		self.strip_unsupported_client_capabilities(&mut init_request.params.capabilities, &ctx);
		self
			.relay
			.send_single(
				JsonRpcRequest::new(RequestId::Number(0), init_request.into()),
				ctx,
				service_name,
				Some(log),
			)
			.await
	}

	async fn send_initialized_notification_single(
		&self,
		parts: Parts,
		service_name: &str,
	) -> Result<Response, UpstreamError> {
		let initialized = rmcp::model::InitializedNotification {
			method: Default::default(),
			extensions: Default::default(),
		};
		let method = initialized.method.as_str().to_string();
		let ctx = IncomingRequestContext::new(&parts);
		let (_, log, _) = mcp::handler::setup_request_log(parts, &method);
		let session_id = self.id.to_string();
		log.non_atomic_mutate(|l| {
			l.method_name = Some(method.clone());
			l.session_id = Some(session_id);
		});

		self
			.relay
			.send_notification_single(initialized.into(), ctx, service_name)
			.await
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
				let mut ctx = IncomingRequestContext::new(&parts);
				let (mut span, log, cel) = mcp::handler::setup_request_log(parts, &method);
				let session_id = self.id.to_string();
				log.non_atomic_mutate(|l| {
					l.method_name = Some(method.clone());
					l.session_id = Some(session_id);
				});
				self.strip_unsupported_client_capabilities_from_meta(&mut r.request, &ctx);
				match &mut r.request {
					ClientRequest::InitializeRequest(ir) => {
						self.strip_unsupported_client_capabilities(&mut ir.params.capabilities, &ctx);

						let pv = ir.params.protocol_version.clone();
						let res = Box::pin(
							self.relay.send_fanout(
								r,
								ctx,
								self
									.relay
									.merge_initialize(pv, self.relay.is_multiplexing()),
							),
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
					ClientRequest::DiscoverRequest(_) => {
						Box::pin(self.relay.send_fanout(
							r,
							ctx,
							self.relay.merge_discover(self.relay.is_multiplexing()),
						))
						.await
					},
					ClientRequest::ListToolsRequest(_) => {
						Box::pin(self.relay.send_fanout(r, ctx, self.relay.merge_tools())).await
					},
					// TODO(keithmattix): should we forward pings or should we do our own independent pings
					// as heuristic for the connection pool (and handle client pings as a local reply from agentgateway)?
					ClientRequest::PingRequest(_) | ClientRequest::SetLevelRequest(_) => {
						Box::pin(self.relay.send_fanout(r, ctx, self.relay.merge_empty())).await
					},
					ClientRequest::SubscriptionsListenRequest(slr) => {
						// Per-target upstream filters are rebuilt from resource_subs, so the request's
						// URIs stay in the client's service+ form and are never sent upstream as-is.
						let client_filter = slr.params.notifications.clone();
						let mut resource_subs = Vec::new();
						for uri in slr
							.params
							.notifications
							.resource_subscriptions
							.as_deref()
							.unwrap_or_default()
						{
							let (service_name, upstream_uri) = self.relay.parse_resource_uri(uri)?;
							self.authorize_resource_request(
								service_name,
								&upstream_uri,
								&method,
								&mut span,
								&log,
								&cel,
							)?;
							resource_subs.push(ResourceSubscription {
								owner: service_name.to_string(),
								client_uri: uri.clone(),
								upstream_uri,
							});
						}
						Box::pin(
							self
								.relay
								.send_subscriptions_listen(r, ctx, client_filter, resource_subs),
						)
						.await
					},
					ClientRequest::ListPromptsRequest(_) => {
						Box::pin(self.relay.send_fanout(r, ctx, self.relay.merge_prompts())).await
					},
					ClientRequest::ListResourcesRequest(_) => {
						Box::pin(self.relay.send_fanout(r, ctx, self.relay.merge_resources())).await
					},
					ClientRequest::ListResourceTemplatesRequest(_) => {
						Box::pin(
							self
								.relay
								.send_fanout(r, ctx, self.relay.merge_resource_templates()),
						)
						.await
					},
					ClientRequest::CallToolRequest(ctr) => {
						let name = ctr.params.name.clone();
						let (service_name, tool) = Box::pin(self.relay.resolve_resource_name(
							ResolveKind::Tool,
							&name,
							&ctx,
						))
						.await?;
						span.rename_span(format!("{method} {service_name}"));
						let call_arguments = ctr.params.arguments.clone();
						log.non_atomic_mutate(|l| {
							l.set_tool(service_name.to_string(), tool.to_string());
							l.capture_call_arguments(call_arguments);
						});
						let tn = tool.to_string();
						ctr.params.name = tn.into();
						Box::pin(self.authorize_with_ctx(
							&service_name,
							mcp::guardrails::methods::TOOLS_CALL,
							&mut ctr.params,
							&mut ctx,
							rbac::ResourceType::Tool(rbac::ResourceId::new(
								service_name.to_string(),
								tool.to_string(),
							)),
							"tool",
							&name,
						))
						.await?;
						Box::pin(
							self
								.relay
								.send_single(r, ctx, &service_name, Some(log.clone())),
						)
						.await
					},
					ClientRequest::GetPromptRequest(gpr) => {
						let name = gpr.params.name.clone();
						let (service_name, prompt) = Box::pin(self.relay.resolve_resource_name(
							ResolveKind::Prompt,
							&name,
							&ctx,
						))
						.await?;
						span.rename_span(format!("{method} {service_name}"));
						log.non_atomic_mutate(|l| {
							l.set_prompt(service_name.to_string(), prompt.to_string());
						});
						gpr.params.name = prompt.to_string();
						Box::pin(self.authorize_with_ctx(
							&service_name,
							mcp::guardrails::methods::PROMPTS_GET,
							&mut gpr.params,
							&mut ctx,
							rbac::ResourceType::Prompt(rbac::ResourceId::new(
								service_name.to_string(),
								prompt.to_string(),
							)),
							"prompt",
							&name,
						))
						.await?;
						Box::pin(self.relay.send_single(r, ctx, &service_name, None)).await
					},
					ClientRequest::ReadResourceRequest(rrr) => {
						let uri = rrr.params.uri.clone();
						let (service_name, original_uri) = self.relay.parse_resource_uri(&uri)?;
						span.rename_span(format!("{method} {service_name}"));
						log.non_atomic_mutate(|l| {
							l.set_resource(service_name.to_string(), original_uri.to_string());
						});
						rrr.params.uri = original_uri.clone();
						Box::pin(self.authorize_with_ctx(
							service_name,
							mcp::guardrails::methods::RESOURCES_READ,
							&mut rrr.params,
							&mut ctx,
							rbac::ResourceType::Resource(rbac::ResourceId::new(
								service_name.to_string(),
								original_uri,
							)),
							"resource",
							&uri,
						))
						.await?;
						Box::pin(self.relay.send_single(r, ctx, service_name, None)).await
					},
					ClientRequest::SubscribeRequest(sr) => {
						let uri = sr.params.uri.clone();
						let (service_name, original_uri) = self.relay.parse_resource_uri(&uri)?;
						self.authorize_resource_request(
							service_name,
							&original_uri,
							&method,
							&mut span,
							&log,
							&cel,
						)?;
						sr.params.uri = original_uri;
						Box::pin(self.relay.send_single(r, ctx, service_name, None)).await
					},
					ClientRequest::UnsubscribeRequest(ur) => {
						let uri = ur.params.uri.clone();
						let (service_name, original_uri) = self.relay.parse_resource_uri(&uri)?;
						self.authorize_resource_request(
							service_name,
							&original_uri,
							&method,
							&mut span,
							&log,
							&cel,
						)?;
						ur.params.uri = original_uri;
						Box::pin(self.relay.send_single(r, ctx, service_name, None)).await
					},

					ClientRequest::ListTasksRequest(_)
					| ClientRequest::GetTaskRequest(_)
					| ClientRequest::GetTaskPayloadRequest(_)
					| ClientRequest::CancelTaskRequest(_) => {
						// TODO(https://github.com/agentgateway/agentgateway/issues/404)
						Err(UpstreamError::InvalidMethod(r.request.method().to_string()))
					},
					ClientRequest::CustomRequest(_) => {
						let method = r.request.method();
						if mcp::is_known_client_request_method(method) {
							Err(UpstreamError::InvalidRequest(format!(
								"invalid params for method: {method}"
							)))
						} else {
							Err(UpstreamError::InvalidMethod(method.to_string()))
						}
					},
					ClientRequest::CompleteRequest(cr) => match &cr.params.r#ref {
						Reference::Prompt(prompt) => {
							let name = prompt.name.clone();
							let (service_name, prompt_name) = Box::pin(
								self.authorize_prompt_request(&name, &method, &mut span, &log, &cel, &ctx),
							)
							.await?;
							cr.params.r#ref = Reference::for_prompt(prompt_name.to_string());
							Box::pin(self.relay.send_single(r, ctx, &service_name, None)).await
						},
						Reference::Resource(resource) => {
							let uri = resource.uri.clone();
							let (service_name, original_uri) = self.relay.parse_resource_uri(&uri)?;
							self.authorize_resource_request(
								service_name,
								&original_uri,
								&method,
								&mut span,
								&log,
								&cel,
							)?;
							cr.params.r#ref = Reference::for_resource(original_uri);
							Box::pin(self.relay.send_single(r, ctx, service_name, None)).await
						},
						_ => Err(UpstreamError::InvalidMethod(method)),
					},
				}
			},
			ClientJsonRpcMessage::Notification(mut r) => {
				let method = match &r.notification {
					ClientNotification::CancelledNotification(r) => r.method.as_str(),
					ClientNotification::ProgressNotification(r) => r.method.as_str(),
					ClientNotification::InitializedNotification(r) => r.method.as_str(),
					ClientNotification::RootsListChangedNotification(r) => r.method.as_str(),
					ClientNotification::TaskStatusNotification(r) => r.method.as_str(),
					ClientNotification::CustomNotification(r) => r.method.as_str(),
				};
				let ctx = IncomingRequestContext::new(&parts);
				let (_span, log, _cel) = mcp::handler::setup_request_log(parts, method);
				let session_id = self.id.to_string();
				log.non_atomic_mutate(|l| {
					l.method_name = Some(method.to_string());
					l.session_id = Some(session_id);
				});
				self.strip_unsupported_client_capabilities_from_meta(&mut r.notification, &ctx);
				// TODO: the notification needs to be fanned out in some cases and sent to a single one in others
				// however, we don't have a way to map to the correct service yet
				Box::pin(self.relay.send_notification(r, ctx)).await
			},

			_ => Err(UpstreamError::InvalidRequest(
				"unsupported message type".to_string(),
			)),
		}
	}

	fn strip_unsupported_client_capabilities(
		&self,
		capabilities: &mut rmcp::model::ClientCapabilities,
		ctx: &IncomingRequestContext,
	) {
		// TODO implement MCP tasks
		capabilities.tasks = None;
		if let Some(extensions) = capabilities.extensions.as_mut() {
			extensions.remove("io.modelcontextprotocol/tasks");
			if extensions.is_empty() {
				capabilities.extensions = None;
			}
		}

		if !mcp::handler::ctx_downstream_modern(ctx) {
			// Legacy clients require reverse JSON-RPC routing for these capabilities.
			capabilities.roots = None;
			capabilities.sampling = None;
			capabilities.elicitation = None;
		}
	}

	fn strip_unsupported_client_capabilities_from_meta<T: GetMeta>(
		&self,
		message: &mut T,
		ctx: &IncomingRequestContext,
	) {
		let Some(mut capabilities) = message.get_meta().client_capabilities() else {
			return;
		};
		self.strip_unsupported_client_capabilities(&mut capabilities, ctx);
		message.get_meta_mut().set_client_capabilities(capabilities);
	}
}

#[derive(Debug)]
pub struct SessionManager {
	encoder: http::sessionpersistence::Encoder,
	sessions: Arc<RwLock<HashMap<String, SessionEntry>>>,
	idle_reaper: OnceLock<tokio::task::AbortHandle>,
}

fn session_id() -> Arc<str> {
	uuid::Uuid::new_v4().to_string().into()
}

impl SessionManager {
	pub fn new(encoder: http::sessionpersistence::Encoder) -> Arc<Self> {
		Arc::new(Self {
			encoder,
			sessions: Arc::new(RwLock::new(HashMap::new())),
			idle_reaper: OnceLock::new(),
		})
	}

	pub fn ensure_idle_running(&self) {
		self
			.idle_reaper
			.get_or_init(|| tokio::spawn(run_idle_reaper(self.sessions.clone())).abort_handle());
	}

	pub fn get_session(&self, id: &str, builder: RelayInputs) -> Option<Session> {
		let mut sessions = self.sessions.write().ok()?;
		let entry = sessions.get_mut(id)?;
		entry.last_access = Instant::now();
		Some(entry.session.clone().with_inputs(builder))
	}

	pub fn get_or_resume_session(
		&self,
		id: &str,
		builder: RelayInputs,
	) -> Result<Option<Session>, mcp::Error> {
		if let Some(s) = self.sessions.write().expect("poisoned").get_mut(id) {
			s.last_access = Instant::now();
			return Ok(Some(s.session.clone().with_inputs(builder)));
		}
		let idle_ttl = builder.backend.session_idle_ttl;
		let d = http::sessionpersistence::SessionState::decode(id, &self.encoder)
			.map_err(|_| mcp::Error::InvalidSessionIdHeader)?;
		let http::sessionpersistence::SessionState::MCP(state) = d else {
			return Ok(None);
		};
		let relay = builder.build_new_connections()?;
		if let Err(err) = relay.set_sessions(state.sessions) {
			warn!("failed to resume session: {err}");
			return Ok(None);
		}

		let sess = Session {
			id: id.into(),
			relay: Arc::new(relay),
			tx: None,
			encoder: self.encoder.clone(),
		};
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(
			id.to_string(),
			SessionEntry {
				session: sess.clone(),
				last_access: Instant::now(),
				idle_ttl,
			},
		);
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

	pub fn insert_session(&self, sess: Session, idle_ttl: Duration) {
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(
			sess.id.to_string(),
			SessionEntry {
				session: sess,
				last_access: Instant::now(),
				idle_ttl,
			},
		);
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
	pub fn create_legacy_session(
		&self,
		relay: Relay,
		idle_ttl: Duration,
	) -> (Session, Receiver<ServerJsonRpcMessage>) {
		let (tx, rx) = tokio::sync::mpsc::channel(64);
		let id = session_id();
		let sess = Session {
			id: id.clone(),
			relay: Arc::new(relay),
			tx: Some(tx),
			encoder: self.encoder.clone(),
		};
		let mut sm = self.sessions.write().expect("write lock");
		sm.insert(
			id.to_string(),
			SessionEntry {
				session: sess.clone(),
				last_access: Instant::now(),
				idle_ttl,
			},
		);
		(sess, rx)
	}

	pub async fn delete_session(&self, id: &str, parts: Parts) -> Option<Response> {
		let sess = {
			let mut sm = self.sessions.write().expect("write lock");
			sm.remove(id)?.session
		};
		// Swallow the error
		sess.delete_session(parts).await.ok()
	}
}

impl Drop for SessionManager {
	fn drop(&mut self) {
		if let Some(abort) = self.idle_reaper.take() {
			abort.abort();
		}
	}
}

async fn run_idle_reaper(sessions: Arc<RwLock<HashMap<String, SessionEntry>>>) {
	let mut ticker = tokio::time::interval(SESSION_REAP_INTERVAL);
	ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	loop {
		ticker.tick().await;
		reap_expired_entries(&sessions);
	}
}

fn reap_expired_entries(sessions: &Arc<RwLock<HashMap<String, SessionEntry>>>) {
	let now = Instant::now();
	let mut guard = sessions.write().expect("write lock");
	let pre = guard.len();
	guard.retain(|_, entry| now.duration_since(entry.last_access) < entry.idle_ttl);
	let post = guard.len();
	if post < pre {
		tracing::debug!("reaped {} sessions", pre - post);
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
		let mut sse = Sse::default().event("message").data(data);
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
	let mut client_info = ClientInfo::default();
	client_info.protocol_version = ProtocolVersion::V_2025_11_25;
	client_info.capabilities = rmcp::model::ClientCapabilities::default();
	client_info.client_info =
		Implementation::new("agentgateway", BuildInfo::new().version.to_string());
	client_info
}
