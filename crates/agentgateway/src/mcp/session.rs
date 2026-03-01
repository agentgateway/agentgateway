use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::SystemTime;

use ::http::StatusCode;
use ::http::header::CONTENT_TYPE;
use ::http::request::Parts;
use agent_core::version::BuildInfo;
use anyhow::anyhow;
use futures_util::StreamExt;
use headers::HeaderMapExt;
use rmcp::model::{
	ClientInfo, ClientJsonRpcMessage, ClientNotification, ClientRequest, ConstString, GetExtensions,
	Implementation, Meta, ProtocolVersion, RequestId, ServerJsonRpcMessage,
};
use rmcp::transport::common::http_header::{EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE};
use sse_stream::{KeepAlive, Sse, SseBody, SseStream};
use tokio::sync::mpsc::{Receiver, Sender};

use opentelemetry::KeyValue;
use opentelemetry::trace::{Event, Link, Status as SpanStatus, TraceContextExt};

use crate::http::Response;
use crate::mcp::handler::{Relay, RelayInputs};
use crate::mcp::mergestream::Messages;
use crate::mcp::streamablehttp::{ServerSseMessage, StreamableHttpPostResponse};
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, MCPOperation, rbac};
use crate::proxy::ProxyError;
use crate::telemetry::log::{SpanWriter, StartedSpan, mcp_gen_ai_operation_name};
use crate::telemetry::operation_spans::{OperationKind, operation_span_fields};
use crate::telemetry::trc::{TraceParent, remote_parent_context};
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
				let network_protocol_version = http_protocol_version(parts.version);
				let method = r.request.method().to_string();
				let ctx = IncomingRequestContext::new(&parts);
				let (_span, log, cel) = mcp::handler::setup_request_log(parts, &method);
				let session_id = self.id.to_string();
				let request_id = r.id.to_string();
				log.non_atomic_mutate(|l| {
					l.method_name = Some(method.clone());
					l.session_id = Some(session_id.clone());
					l.jsonrpc_request_id = Some(request_id.clone());
				});

				// Phase 1: capture context for MCP operation span before ctx/r are consumed.
				let span_writer = ctx.span_writer();
				let span_start = SystemTime::now();
				let mut lifecycle = McpLifecycle::new(span_start);
				let mut mcp_span_target = precompute_mcp_span_target(&self.relay, &r.request);
				let mut mcp_resource_uri: Option<String> = None;
				let mut fanout_mode = false;
				// Phase 3: unique per-turn identifier (gateway.turn.id).
				let turn_id = uuid::Uuid::new_v4().to_string();

				// Phase 2: extract inbound _meta.traceparent for a link, then inject outbound.
				// The extraction must happen before injection overwrites the field.
				let inbound_meta_tp: Option<TraceParent> = r
					.request
					.extensions()
					.get::<Meta>()
					.and_then(|m| m.0.get("traceparent"))
					.and_then(|v| v.as_str())
					.and_then(|s| TraceParent::try_from(s).ok());
				let mut operation_span = span_writer.as_ref().map(|sw| {
					start_mcp_operation_span(
						sw,
						&method,
						mcp_span_target.as_deref(),
						inbound_meta_tp.as_ref(),
					)
				});
				let operation_traceparent = operation_span
					.as_ref()
					.map(|span| span.traceparent().clone());
				{
					let meta: &mut Meta = r.request.extensions_mut().get_or_insert_default();
					// Security policy: strip inbound baggage before forwarding to upstream.
					meta.0.remove("baggage");
					// Inject the operation span traceparent so upstream calls are direct children.
					if let Some(operation_tp) = operation_traceparent.as_ref() {
						meta.0.insert(
							"traceparent".to_string(),
							serde_json::Value::String(operation_tp.to_string()),
						);
					}
				}

				let mcp_result = match &mut r.request {
					ClientRequest::InitializeRequest(ir) => {
						// Currently, we cannot support roots until we have a mapping of downstream and upstream ID.
						// However, the clients can tell the server they support roots.
						// Instead, we hijack this to tell them not to so they do not send requests that we cannot
						// actually support
						// This could probably be more easily done without multiplexing but for now neither supports.
						ir.params.capabilities.roots = None;

						let pv = ir.params.protocol_version.clone();
						log.non_atomic_mutate(|l| {
							l.protocol_version = Some(pv.to_string());
						});
						fanout_mode = true;
						lifecycle.mark_fanout();
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
						fanout_mode = true;
						lifecycle.mark_fanout();
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_tools(cel))
							.await
					},
					ClientRequest::PingRequest(_) | ClientRequest::SetLevelRequest(_) => {
						fanout_mode = true;
						lifecycle.mark_fanout();
						self
							.relay
							.send_fanout(r, ctx, self.relay.merge_empty())
							.await
					},
					ClientRequest::ListPromptsRequest(_) => {
						log.non_atomic_mutate(|l| {
							l.resource = Some(MCPOperation::Prompt);
						});
						fanout_mode = true;
						lifecycle.mark_fanout();
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
							fanout_mode = true;
							lifecycle.mark_fanout();
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
							fanout_mode = true;
							lifecycle.mark_fanout();
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
						match self.relay.parse_resource_name(&name) {
							Ok((service_name, tool)) => {
								log.non_atomic_mutate(|l| {
									l.resource_name = Some(tool.to_string());
									l.target_name = Some(service_name.to_string());
									l.resource = Some(MCPOperation::Tool);
								});
								mcp_span_target = Some(service_name.to_string());
								if !self.relay.policies.validate(
									&rbac::ResourceType::Tool(rbac::ResourceId::new(
										service_name.to_string(),
										tool.to_string(),
									)),
									&cel,
								) {
									Err(UpstreamError::Authorization {
										resource_type: "tool".to_string(),
										resource_name: name.to_string(),
									})
								} else {
									let tn = tool.to_string();
									ctr.params.name = tn.into();
									self.relay.send_single(r, ctx, service_name).await
								}
							},
							Err(e) => Err(e),
						}
					},
					ClientRequest::GetPromptRequest(gpr) => {
						let name = gpr.params.name.clone();
						match self.relay.parse_resource_name(&name) {
							Ok((service_name, prompt)) => {
								log.non_atomic_mutate(|l| {
									l.target_name = Some(service_name.to_string());
									l.resource_name = Some(prompt.to_string());
									l.resource = Some(MCPOperation::Prompt);
								});
								mcp_span_target = Some(service_name.to_string());
								if !self.relay.policies.validate(
									&rbac::ResourceType::Prompt(rbac::ResourceId::new(
										service_name.to_string(),
										prompt.to_string(),
									)),
									&cel,
								) {
									Err(UpstreamError::Authorization {
										resource_type: "prompt".to_string(),
										resource_name: name.to_string(),
									})
								} else {
									gpr.params.name = prompt.to_string();
									self.relay.send_single(r, ctx, service_name).await
								}
							},
							Err(e) => Err(e),
						}
					},
					ClientRequest::ReadResourceRequest(rrr) => {
						if let Some(service_name) = self.relay.default_target_name() {
							let uri = rrr.params.uri.clone();
							log.non_atomic_mutate(|l| {
								l.target_name = Some(service_name.to_string());
								l.resource_name = Some(uri.to_string());
								l.resource = Some(MCPOperation::Resource);
							});
							mcp_span_target = Some(service_name.to_string());
							mcp_resource_uri = sanitize_mcp_resource_uri(uri.as_str());
							if !self.relay.policies.validate(
								&rbac::ResourceType::Resource(rbac::ResourceId::new(
									service_name.to_string(),
									uri.to_string(),
								)),
								&cel,
							) {
								Err(UpstreamError::Authorization {
									resource_type: "resource".to_string(),
									resource_name: uri.to_string(),
								})
							} else {
								self.relay.send_single_without_multiplexing(r, ctx).await
							}
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
				};
				let retry_count = 0;
				lifecycle.mark_retry(retry_count);
				lifecycle.mark_finalized();

				// Phase 1+2+3: write typed MCP operation span with caller trace link and turn semantics.
				if let Some(operation_span) = operation_span.as_mut() {
					finalize_mcp_operation_span(
						operation_span,
						McpOperationSpanInput {
							method: &method,
							target: mcp_span_target.as_deref(),
							session_id: &session_id,
							request_id: &request_id,
							jsonrpc_protocol_version: Some("2.0"),
							mcp_resource_uri: mcp_resource_uri.as_deref(),
							network_protocol_version,
							turn_id: &turn_id,
							gen_ai_op: mcp_gen_ai_operation_name(Some(&method)),
							result: &mcp_result,
							fanout_mode,
							retry_count,
							lifecycle,
						},
					);
				}
				mcp_result
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

/// Map an `UpstreamError` to a low-cardinality `error.type` string.
fn mcp_error_type(err: &UpstreamError) -> &'static str {
	match err {
		UpstreamError::Authorization { .. } => "authorization",
		UpstreamError::InvalidRequest(_) => "invalid_request",
		UpstreamError::InvalidMethod(_) | UpstreamError::InvalidMethodWithMultiplexing(_) => {
			"invalid_method"
		},
		_ => "upstream_error",
	}
}

fn mcp_rpc_response_status_code(result: &Result<Response, UpstreamError>) -> Option<i64> {
	match result.as_ref().err()? {
		// Authorization failures are intentionally surfaced as INVALID_PARAMS to avoid
		// exposing resource existence details.
		UpstreamError::Authorization { .. } => Some(-32602),
		// These are transformed into ProxyError::MCP(SendError), which maps to
		// JSON-RPC INTERNAL_ERROR in proxy response encoding.
		UpstreamError::InvalidRequest(_)
		| UpstreamError::InvalidMethod(_)
		| UpstreamError::InvalidMethodWithMultiplexing(_)
		| UpstreamError::Send
		| UpstreamError::Recv => Some(-32603),
		// Non JSON-RPC direct upstream/proxy failures do not have a stable rpc code.
		UpstreamError::Http(_) | UpstreamError::Proxy(_) => None,
		_ => Some(-32603),
	}
}

fn sanitize_mcp_resource_uri(uri: &str) -> Option<String> {
	const MAX_URI_BYTES: usize = 256;
	if uri.is_empty() || uri.len() > MAX_URI_BYTES {
		return None;
	}
	// Require a URI-ish shape and avoid control chars.
	if !uri.contains(':') || uri.chars().any(char::is_control) {
		return None;
	}
	Some(uri.to_string())
}

fn http_protocol_version(version: ::http::Version) -> Option<&'static str> {
	match version {
		::http::Version::HTTP_11 => Some("1.1"),
		::http::Version::HTTP_2 => Some("2"),
		_ => None,
	}
}

/// Write a typed MCP operation span covering the full lifecycle of one JSON-RPC operation.
///
/// Span name format: `"{method} {target}"` when a target is known, `"{method}"` otherwise.
/// SpanKind is CLIENT (the gateway acting as MCP client toward upstream servers).
/// If `inbound_meta_tp` is set (the downstream caller embedded a traceparent in `_meta`),
/// it is added as a span link so the caller's trace can be correlated.
#[derive(Clone)]
struct McpLifecycle {
	received_at: SystemTime,
	fanout_at: Option<SystemTime>,
	retry_at: Option<SystemTime>,
	finalized_at: Option<SystemTime>,
}

impl McpLifecycle {
	fn new(received_at: SystemTime) -> Self {
		Self {
			received_at,
			fanout_at: None,
			retry_at: None,
			finalized_at: None,
		}
	}

	fn mark_fanout(&mut self) {
		if self.fanout_at.is_none() {
			self.fanout_at = Some(SystemTime::now());
		}
	}

	fn mark_retry(&mut self, retry_count: u32) {
		if retry_count > 0 || self.retry_at.is_none() {
			self.retry_at = Some(SystemTime::now());
		}
	}

	fn mark_finalized(&mut self) {
		self.finalized_at = Some(SystemTime::now());
	}

	fn fanout_checkpoint(&self) -> SystemTime {
		self.fanout_at.unwrap_or(self.received_at)
	}

	fn retry_checkpoint(&self) -> SystemTime {
		self
			.retry_at
			.or(self.finalized_at)
			.unwrap_or(self.received_at)
	}

	fn finalized_checkpoint(&self) -> SystemTime {
		self.finalized_at.unwrap_or(self.received_at)
	}
}

fn precompute_mcp_span_target(relay: &Relay, request: &ClientRequest) -> Option<String> {
	match request {
		ClientRequest::CallToolRequest(ctr) => relay
			.parse_resource_name(&ctr.params.name)
			.ok()
			.map(|(service_name, _)| service_name.to_string()),
		ClientRequest::GetPromptRequest(gpr) => relay
			.parse_resource_name(&gpr.params.name)
			.ok()
			.map(|(service_name, _)| service_name.to_string()),
		ClientRequest::ReadResourceRequest(_) => {
			relay.default_target_name().map(|name| name.to_string())
		},
		_ => None,
	}
}

fn start_mcp_operation_span(
	sw: &SpanWriter,
	method: &str,
	target: Option<&str>,
	inbound_meta_tp: Option<&TraceParent>,
) -> StartedSpan {
	let name = match target {
		Some(target) if !target.is_empty() => format!("{method} {target}"),
		_ => method.to_string(),
	};
	sw.start(name, |mut sb| {
		// Add a link from the downstream caller's trace if they propagated one via _meta.
		if let Some(tp) = inbound_meta_tp {
			let caller_ctx = remote_parent_context(tp);
			let sc = caller_ctx.span().span_context().clone();
			if sc.is_valid() {
				sb = sb.with_links(vec![Link::new(sc, vec![], 0)]);
			}
		}
		sb
	})
}

struct McpOperationSpanInput<'a> {
	method: &'a str,
	target: Option<&'a str>,
	session_id: &'a str,
	request_id: &'a str,
	jsonrpc_protocol_version: Option<&'a str>,
	mcp_resource_uri: Option<&'a str>,
	network_protocol_version: Option<&'a str>,
	turn_id: &'a str,
	gen_ai_op: Option<&'static str>,
	result: &'a Result<Response, UpstreamError>,
	fanout_mode: bool,
	retry_count: u32,
	lifecycle: McpLifecycle,
}

fn finalize_mcp_operation_span(operation_span: &mut StartedSpan, input: McpOperationSpanInput<'_>) {
	let McpOperationSpanInput {
		method,
		target,
		session_id,
		request_id,
		jsonrpc_protocol_version,
		mcp_resource_uri,
		network_protocol_version,
		turn_id,
		gen_ai_op,
		result,
		fanout_mode,
		retry_count,
		lifecycle,
	} = input;
	let error_type = result.as_ref().err().map(mcp_error_type);
	let is_error = error_type.is_some();
	let rpc_response_status_code = mcp_rpc_response_status_code(result);
	let fields = operation_span_fields(OperationKind::Mcp {
		method,
		target,
		session_id,
		request_id,
		jsonrpc_protocol_version,
		rpc_response_status_code,
		resource_uri: mcp_resource_uri,
		network_protocol_name: Some("http"),
		network_protocol_version,
		network_transport: Some("tcp"),
		turn_id,
		gen_ai_operation: gen_ai_op,
		fanout_mode,
		retry_count,
	});

	let mut attrs = fields.attrs;
	if let Some(et) = error_type {
		attrs.push(KeyValue::new("error.type", et));
	}
	operation_span.set_attributes(attrs);
	for event in mcp_lifecycle_events(
		method,
		fanout_mode,
		retry_count,
		is_error,
		error_type,
		&lifecycle,
	) {
		operation_span.add_event(event);
	}
	if is_error {
		operation_span.set_status(SpanStatus::error(error_type.unwrap_or("error")));
	}
	operation_span.end();
}

fn mcp_lifecycle_events(
	method: &str,
	fanout_mode: bool,
	retry_count: u32,
	is_error: bool,
	error_type: Option<&'static str>,
	lifecycle: &McpLifecycle,
) -> Vec<Event> {
	let mut out = vec![
		Event::new(
			"gateway.mcp.lifecycle.received",
			lifecycle.received_at,
			vec![KeyValue::new("mcp.method.name", method.to_string())],
			0,
		),
		Event::new(
			"gateway.mcp.lifecycle.fanout",
			lifecycle.fanout_checkpoint(),
			vec![KeyValue::new("gateway.mcp.lifecycle.fanout", fanout_mode)],
			0,
		),
		Event::new(
			"gateway.mcp.lifecycle.retry",
			lifecycle.retry_checkpoint(),
			vec![KeyValue::new(
				"gateway.mcp.lifecycle.retry.count",
				i64::from(retry_count),
			)],
			0,
		),
	];
	let mut finalized_attrs = vec![KeyValue::new(
		"gateway.mcp.lifecycle.result",
		if is_error { "error" } else { "ok" },
	)];
	if let Some(et) = error_type {
		finalized_attrs.push(KeyValue::new("error.type", et));
	}
	out.push(Event::new(
		"gateway.mcp.lifecycle.finalized",
		lifecycle.finalized_checkpoint(),
		finalized_attrs,
		0,
	));
	out
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
	use opentelemetry::trace::{SpanKind, Status as OTelStatus, TracerProvider};
	use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
	use opentelemetry_sdk::trace::{SdkTracerProvider, SimpleSpanProcessor, SpanData};
	use std::sync::{Arc, Mutex};
	use std::time::Duration;

	#[derive(Clone, Debug, Default)]
	struct CaptureExporter {
		spans: Arc<Mutex<Vec<SpanData>>>,
	}

	impl CaptureExporter {
		fn get_finished_spans(&self) -> Vec<SpanData> {
			self.spans.lock().expect("spans lock").clone()
		}
	}

	impl opentelemetry_sdk::trace::SpanExporter for CaptureExporter {
		fn export(
			&self,
			mut batch: Vec<SpanData>,
		) -> impl std::future::Future<Output = OTelSdkResult> + Send {
			let spans = self.spans.clone();
			async move {
				spans
					.lock()
					.map_err(|e| OTelSdkError::InternalFailure(e.to_string()))?
					.append(&mut batch);
				Ok(())
			}
		}

		fn shutdown_with_timeout(&mut self, _timeout: Duration) -> OTelSdkResult {
			Ok(())
		}
	}

	fn make_span_writer_and_exporter() -> (SpanWriter, CaptureExporter) {
		let exporter = CaptureExporter::default();
		let provider = SdkTracerProvider::builder()
			.with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
			.build();
		let sdk_tracer = provider.tracer("mcp-session-tests");
		let tracer = Arc::new(crate::telemetry::trc::Tracer {
			tracer: Arc::new(sdk_tracer),
			provider,
			fields: Arc::new(crate::telemetry::log::LoggingFields::default()),
		});
		let request_parent = TraceParent {
			version: 0,
			trace_id: 0xabc1,
			span_id: 0xabc2,
			flags: 0x01,
		};
		let writer = SpanWriter::for_test(request_parent, tracer);
		(writer, exporter)
	}

	#[test]
	fn mcp_span_name_includes_target_when_present() {
		// start_mcp_operation_span uses this same format.
		let method = "tools/call";
		let target = Some("my-backend");
		let name = match target {
			Some(t) if !t.is_empty() => format!("{method} {t}"),
			_ => method.to_string(),
		};
		assert_eq!(name, "tools/call my-backend");
	}

	#[test]
	fn mcp_span_name_omits_target_when_none() {
		let method = "tools/list";
		let target: Option<&str> = None;
		let name = match target {
			Some(t) if !t.is_empty() => format!("{method} {t}"),
			_ => method.to_string(),
		};
		assert_eq!(name, "tools/list");
	}

	#[test]
	fn mcp_error_type_maps_variants() {
		assert_eq!(
			mcp_error_type(&UpstreamError::Authorization {
				resource_type: "tool".into(),
				resource_name: "x".into(),
			}),
			"authorization"
		);
		assert_eq!(
			mcp_error_type(&UpstreamError::InvalidRequest("bad".into())),
			"invalid_request"
		);
		assert_eq!(
			mcp_error_type(&UpstreamError::InvalidMethod("m".into())),
			"invalid_method"
		);
		assert_eq!(
			mcp_error_type(&UpstreamError::InvalidMethodWithMultiplexing("m".into())),
			"invalid_method"
		);
	}

	#[test]
	fn mcp_rpc_response_status_code_maps_expected_variants() {
		let ok: Result<Response, UpstreamError> = Ok(Response::new(crate::http::Body::empty()));
		assert_eq!(mcp_rpc_response_status_code(&ok), None);

		let authorization = Err(UpstreamError::Authorization {
			resource_type: "tool".into(),
			resource_name: "x".into(),
		});
		assert_eq!(mcp_rpc_response_status_code(&authorization), Some(-32602));

		let invalid_method = Err(UpstreamError::InvalidMethod("m".into()));
		assert_eq!(mcp_rpc_response_status_code(&invalid_method), Some(-32603));
	}

	#[test]
	fn sanitize_mcp_resource_uri_enforces_basic_controls() {
		assert_eq!(
			sanitize_mcp_resource_uri("memo://insights"),
			Some("memo://insights".to_string())
		);
		assert_eq!(sanitize_mcp_resource_uri(""), None);
		assert_eq!(sanitize_mcp_resource_uri("just-text-no-colon"), None);
		assert_eq!(sanitize_mcp_resource_uri("memo://bad\nuri"), None);
		assert_eq!(sanitize_mcp_resource_uri(&"a".repeat(257)), None);
	}

	// Phase 2: _meta trace context propagation tests

	#[test]
	fn inbound_meta_traceparent_parsed_from_valid_string() {
		let valid = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
		let tp = TraceParent::try_from(valid);
		assert!(tp.is_ok(), "valid traceparent must parse");
		let tp = tp.unwrap();
		assert_eq!(tp.trace_id, 0x4bf92f3577b34da6a3ce929d0e0e4736u128);
		assert_eq!(tp.span_id, 0x00f067aa0ba902b7u64);
	}

	#[test]
	fn inbound_meta_traceparent_rejects_invalid_string() {
		let bad = "not-a-traceparent";
		assert!(
			TraceParent::try_from(bad).is_err(),
			"invalid traceparent must fail"
		);
	}

	#[test]
	fn span_writer_traceparent_str_formats_as_w3c() {
		// Build a minimal TraceParent and check the string round-trips through
		// the W3C format: "00-{trace_id:032x}-{span_id:016x}-{flags:02x}"
		let tp =
			TraceParent::try_from("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01").unwrap();
		let s = tp.to_string();
		assert_eq!(s, "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01");
	}

	#[test]
	fn meta_baggage_field_name_is_stable() {
		// Confirm the field name constant used in baggage stripping matches expectations.
		let mut m = Meta::default();
		m.0.insert(
			"baggage".to_string(),
			serde_json::Value::String("k=v".to_string()),
		);
		m.0.insert(
			"other".to_string(),
			serde_json::Value::String("stays".to_string()),
		);
		m.0.remove("baggage");
		assert!(!m.0.contains_key("baggage"), "baggage must be removed");
		assert!(m.0.contains_key("other"), "other keys must be preserved");
	}

	#[test]
	fn mcp_operation_span_emits_lifecycle_events_for_success() {
		let (writer, exporter) = make_span_writer_and_exporter();
		let result: Result<Response, UpstreamError> = Ok(Response::new(crate::http::Body::empty()));
		let mut operation_span =
			start_mcp_operation_span(&writer, "tools/list", Some("upstream-a"), None);
		let operation_tp = operation_span.traceparent().clone();
		finalize_mcp_operation_span(
			&mut operation_span,
			McpOperationSpanInput {
				method: "tools/list",
				target: Some("upstream-a"),
				session_id: "session-1",
				request_id: "req-1",
				jsonrpc_protocol_version: Some("2.0"),
				mcp_resource_uri: None,
				network_protocol_version: Some("1.1"),
				turn_id: "turn-1",
				gen_ai_op: None,
				result: &result,
				fanout_mode: true,
				retry_count: 0,
				lifecycle: McpLifecycle::new(SystemTime::now()),
			},
		);

		let spans = exporter.get_finished_spans();
		assert_eq!(spans.len(), 1);
		let span = &spans[0];
		assert_eq!(span.name.as_ref(), "tools/list upstream-a");
		assert_eq!(span.span_kind, SpanKind::Client);
		assert_eq!(
			u128::from_be_bytes(span.span_context.trace_id().to_bytes()),
			operation_tp.trace_id
		);
		assert_eq!(u64::from_be_bytes(span.parent_span_id.to_bytes()), 0xabc2);
		assert!(span.parent_span_is_remote);
		let names = span
			.events
			.events
			.iter()
			.map(|e| e.name.as_ref().to_string())
			.collect::<Vec<_>>();
		assert_eq!(
			names,
			vec![
				"gateway.mcp.lifecycle.received".to_string(),
				"gateway.mcp.lifecycle.fanout".to_string(),
				"gateway.mcp.lifecycle.retry".to_string(),
				"gateway.mcp.lifecycle.finalized".to_string(),
			]
		);
		assert!(
			span
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "gateway.mcp.lifecycle.fanout" && kv.value.as_str() == "true")
		);
		assert!(
			span.attributes.iter().any(
				|kv| kv.key.as_str() == "gateway.mcp.lifecycle.retry.count" && kv.value.as_str() == "0"
			)
		);
		assert!(
			span
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "jsonrpc.protocol.version" && kv.value.as_str() == "2.0")
		);
		assert!(
			span
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "network.protocol.name" && kv.value.as_str() == "http")
		);
		assert!(
			span
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "network.transport" && kv.value.as_str() == "tcp")
		);
	}

	#[test]
	fn mcp_operation_span_links_inbound_traceparent() {
		let (writer, exporter) = make_span_writer_and_exporter();
		let result: Result<Response, UpstreamError> = Ok(Response::new(crate::http::Body::empty()));
		let inbound = TraceParent {
			version: 0,
			trace_id: 0x4444,
			span_id: 0x2222,
			flags: 0x01,
		};
		let mut operation_span =
			start_mcp_operation_span(&writer, "tools/call", Some("upstream-a"), Some(&inbound));
		finalize_mcp_operation_span(
			&mut operation_span,
			McpOperationSpanInput {
				method: "tools/call",
				target: Some("upstream-a"),
				session_id: "session-1",
				request_id: "req-1",
				jsonrpc_protocol_version: Some("2.0"),
				mcp_resource_uri: None,
				network_protocol_version: Some("1.1"),
				turn_id: "turn-1",
				gen_ai_op: Some("execute_tool"),
				result: &result,
				fanout_mode: false,
				retry_count: 0,
				lifecycle: McpLifecycle::new(SystemTime::now()),
			},
		);
		let spans = exporter.get_finished_spans();
		assert_eq!(spans.len(), 1);
		let span = &spans[0];
		assert_eq!(span.links.links.len(), 1);
		let link_ctx = &span.links.links[0].span_context;
		assert_eq!(
			u128::from_be_bytes(link_ctx.trace_id().to_bytes()),
			inbound.trace_id
		);
		assert_eq!(
			u64::from_be_bytes(link_ctx.span_id().to_bytes()),
			inbound.span_id
		);
		assert!(link_ctx.is_remote());
	}

	#[test]
	fn mcp_operation_span_emits_error_status_and_finalized_error_event() {
		let (writer, exporter) = make_span_writer_and_exporter();
		let result: Result<Response, UpstreamError> = Err(UpstreamError::Authorization {
			resource_type: "tool".to_string(),
			resource_name: "echo".to_string(),
		});
		let mut operation_span =
			start_mcp_operation_span(&writer, "tools/call", Some("upstream-a"), None);
		finalize_mcp_operation_span(
			&mut operation_span,
			McpOperationSpanInput {
				method: "tools/call",
				target: Some("upstream-a"),
				session_id: "session-1",
				request_id: "req-2",
				jsonrpc_protocol_version: Some("2.0"),
				mcp_resource_uri: None,
				network_protocol_version: Some("1.1"),
				turn_id: "turn-2",
				gen_ai_op: Some("execute_tool"),
				result: &result,
				fanout_mode: false,
				retry_count: 0,
				lifecycle: McpLifecycle::new(SystemTime::now()),
			},
		);

		let spans = exporter.get_finished_spans();
		assert_eq!(spans.len(), 1);
		let span = &spans[0];
		assert_eq!(
			span.status,
			OTelStatus::Error {
				description: "authorization".into(),
			}
		);
		assert!(
			span
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "error.type" && kv.value.as_str() == "authorization")
		);
		assert!(
			span
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "rpc.response.status_code" && kv.value.as_str() == "-32602")
		);
		let finalized = span
			.events
			.events
			.iter()
			.find(|e| e.name.as_ref() == "gateway.mcp.lifecycle.finalized")
			.expect("finalized lifecycle event");
		assert!(
			finalized
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "gateway.mcp.lifecycle.result" && kv.value.as_str() == "error")
		);
		assert!(
			finalized
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "error.type" && kv.value.as_str() == "authorization")
		);
	}

	#[test]
	fn mcp_operation_span_uses_invalid_method_error_type() {
		let (writer, exporter) = make_span_writer_and_exporter();
		let result: Result<Response, UpstreamError> =
			Err(UpstreamError::InvalidMethod("bad".to_string()));
		let mut operation_span = start_mcp_operation_span(&writer, "tools/call", None, None);
		finalize_mcp_operation_span(
			&mut operation_span,
			McpOperationSpanInput {
				method: "tools/call",
				target: None,
				session_id: "session-1",
				request_id: "req-3",
				jsonrpc_protocol_version: Some("2.0"),
				mcp_resource_uri: None,
				network_protocol_version: Some("1.1"),
				turn_id: "turn-3",
				gen_ai_op: Some("execute_tool"),
				result: &result,
				fanout_mode: false,
				retry_count: 0,
				lifecycle: McpLifecycle::new(SystemTime::now()),
			},
		);
		let spans = exporter.get_finished_spans();
		assert_eq!(spans.len(), 1);
		let span = &spans[0];
		assert!(
			span
				.attributes
				.iter()
				.any(|kv| kv.key.as_str() == "error.type" && kv.value.as_str() == "invalid_method")
		);
	}
}
