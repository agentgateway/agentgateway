use std::borrow::Cow;
use std::sync::Arc;

use crate::cel::RequestSnapshot;
use crate::http::Response;
use crate::http::jwt::Claims;
use crate::http::sessionpersistence::MCPSession;
use crate::mcp;
use crate::mcp::mergestream::MergeFn;
use crate::mcp::rbac::{CelExecWrapper, Identity, McpAuthorizationSet};
use crate::mcp::router::McpBackendGroup;
use crate::mcp::streamablehttp::ServerSseMessage;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, MCPInfo, mergestream, rbac, upstream};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::AsyncLog;
use crate::telemetry::trc::TraceParent;
use agent_core::trcng;
use futures_core::Stream;
use http::StatusCode;
use http::request::Parts;
use itertools::Itertools;
use opentelemetry::global::BoxedSpan;
use opentelemetry::trace::{SpanContext, SpanKind, TraceContextExt, TraceState};
use opentelemetry::{Context, TraceFlags};
use rmcp::ErrorData;
use rmcp::model::{
	ClientNotification, ClientRequest, Implementation, JsonRpcNotification, JsonRpcRequest,
	ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, Prompt,
	PromptsCapability, ProtocolVersion, RequestId, ResourcesCapability, ServerCapabilities,
	ServerInfo, ServerJsonRpcMessage, ServerResult, Tool, ToolsCapability,
};

const DELIMITER: &str = "_";

fn resource_name(default_target_name: Option<&String>, target: &str, name: &str) -> String {
	if default_target_name.is_none() {
		format!("{target}{DELIMITER}{name}")
	} else {
		name.to_string()
	}
}

#[derive(Debug, Clone)]
pub struct Relay {
	upstreams: Arc<upstream::UpstreamGroup>,
	pub policies: McpAuthorizationSet,
}

pub struct RelayInputs {
	pub backend: McpBackendGroup,
	pub policies: McpAuthorizationSet,
	pub client: PolicyClient,
}

impl RelayInputs {
	pub fn build_new_connections(self) -> Result<Relay, mcp::Error> {
		Relay::new(self.backend, self.policies, self.client)
	}
}

impl Relay {
	pub fn new(
		backend: McpBackendGroup,
		policies: McpAuthorizationSet,
		client: PolicyClient,
	) -> Result<Self, mcp::Error> {
		Ok(Self {
			upstreams: Arc::new(upstream::UpstreamGroup::new(client, backend)?),
			policies,
		})
	}
	pub fn with_policies(&self, policies: McpAuthorizationSet) -> Self {
		Self {
			upstreams: self.upstreams.clone(),
			policies,
		}
	}

	pub fn parse_resource_name<'a, 'b: 'a>(
		&'a self,
		res: &'b str,
	) -> Result<(&'a str, &'b str), UpstreamError> {
		if let Some(default) = self.upstreams.default_target_name.as_ref() {
			Ok((default.as_str(), res))
		} else {
			res
				.split_once(DELIMITER)
				.ok_or(UpstreamError::InvalidRequest(
					"invalid resource name".to_string(),
				))
		}
	}

	pub fn get_sessions(&self) -> Option<Vec<MCPSession>> {
		let mut sessions = Vec::with_capacity(self.upstreams.size());
		for (_, us) in self.upstreams.iter_named() {
			sessions.push(us.get_session_state()?);
		}
		Some(sessions)
	}

	pub fn set_sessions(&self, sessions: Vec<MCPSession>) {
		for ((_, us), session) in self.upstreams.iter_named().zip(sessions) {
			us.set_session_id(&session.session, session.backend);
		}
	}
	pub fn count(&self) -> usize {
		self.upstreams.size()
	}

	pub fn is_multiplexing(&self) -> bool {
		self.upstreams.is_multiplexing
	}
	pub fn default_target_name(&self) -> Option<String> {
		self.upstreams.default_target_name.clone()
	}

	pub fn merge_tools(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.upstreams.default_target_name.clone();
		Box::new(move |streams| {
			let tools = streams
				.into_iter()
				.flat_map(|(server_name, s)| {
					let tools = match s {
						ServerResult::ListToolsResult(ltr) => ltr.tools,
						_ => vec![],
					};
					tools
						.into_iter()
						// Apply authorization policies, filtering tools that are not allowed.
						.filter(|t| {
							policies.validate(
								&rbac::ResourceType::Tool(rbac::ResourceId::new(
									server_name.to_string(),
									t.name.to_string(),
								)),
								&cel,
							)
						})
						// Rename to handle multiplexing
						.map(|t| Tool {
							name: Cow::Owned(resource_name(
								default_target_name.as_ref(),
								server_name.as_str(),
								&t.name,
							)),
							..t
						})
						.collect_vec()
				})
				.collect_vec();
			Ok(
				ListToolsResult {
					tools,
					next_cursor: None,
					meta: None,
				}
				.into(),
			)
		})
	}

	pub fn merge_initialize(&self, pv: ProtocolVersion, multiplexing: bool) -> Box<MergeFn> {
		Box::new(move |s| {
			if !multiplexing {
				// Happy case: we can forward everything
				let (_, ServerResult::InitializeResult(ir)) = s.into_iter().next().unwrap() else {
					return Ok(Self::get_info(pv, multiplexing).into());
				};
				return Ok(ir.clone().into());
			}

			// Multiplexing is more complex. We need to find the lowest protocol version that all servers support.
			let lowest_version = s
				.into_iter()
				.flat_map(|(_, v)| match v {
					ServerResult::InitializeResult(r) => Some(r.protocol_version),
					_ => None,
				})
				.min_by_key(|i| i.to_string())
				.unwrap_or(pv);
			// For now, we just send our own info. In the future, we should merge the results from each upstream.
			Ok(Self::get_info(lowest_version, multiplexing).into())
		})
	}

	pub fn merge_prompts(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.upstreams.default_target_name.clone();
		Box::new(move |streams| {
			let prompts = streams
				.into_iter()
				.flat_map(|(server_name, s)| {
					let prompts = match s {
						ServerResult::ListPromptsResult(lpr) => lpr.prompts,
						_ => vec![],
					};
					prompts
						.into_iter()
						.filter(|p| {
							policies.validate(
								&rbac::ResourceType::Prompt(rbac::ResourceId::new(
									server_name.to_string(),
									p.name.to_string(),
								)),
								&cel,
							)
						})
						.map(|p| Prompt {
							name: resource_name(default_target_name.as_ref(), server_name.as_str(), &p.name),
							..p
						})
						.collect_vec()
				})
				.collect_vec();
			Ok(
				ListPromptsResult {
					prompts,
					next_cursor: None,
					meta: None,
				}
				.into(),
			)
		})
	}
	pub fn merge_resources(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		Box::new(move |streams| {
			let resources = streams
				.into_iter()
				.flat_map(|(server_name, s)| {
					let resources = match s {
						ServerResult::ListResourcesResult(lrr) => lrr.resources,
						_ => vec![],
					};
					resources
						.into_iter()
						.filter(|r| {
							policies.validate(
								&rbac::ResourceType::Resource(rbac::ResourceId::new(
									server_name.to_string(),
									r.uri.to_string(),
								)),
								&cel,
							)
						})
						// TODO(https://github.com/agentgateway/agentgateway/issues/404) map this to the service name,
						// if we add support for multiple services.
						.collect_vec()
				})
				.collect_vec();
			Ok(
				ListResourcesResult {
					resources,
					next_cursor: None,
					meta: None,
				}
				.into(),
			)
		})
	}
	pub fn merge_resource_templates(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		Box::new(move |streams| {
			let resource_templates = streams
				.into_iter()
				.flat_map(|(server_name, s)| {
					let resource_templates = match s {
						ServerResult::ListResourceTemplatesResult(lrr) => lrr.resource_templates,
						_ => vec![],
					};
					resource_templates
						.into_iter()
						.filter(|rt| {
							policies.validate(
								&rbac::ResourceType::Resource(rbac::ResourceId::new(
									server_name.to_string(),
									rt.uri_template.to_string(),
								)),
								&cel,
							)
						})
						// TODO(https://github.com/agentgateway/agentgateway/issues/404) map this to the service name,
						// if we add support for multiple services.
						.collect_vec()
				})
				.collect_vec();
			Ok(
				ListResourceTemplatesResult {
					resource_templates,
					next_cursor: None,
					meta: None,
				}
				.into(),
			)
		})
	}
	pub fn merge_empty(&self) -> Box<MergeFn> {
		Box::new(move |_| Ok(rmcp::model::ServerResult::empty(())))
	}
	pub async fn send_single(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		service_name: &str,
	) -> Result<Response, UpstreamError> {
		let id = r.id.clone();
		let Ok(us) = self.upstreams.get(service_name) else {
			return Err(UpstreamError::InvalidRequest(format!(
				"unknown service {service_name}"
			)));
		};
		let stream = us.generic_stream(r, &ctx).await?;

		messages_to_response(id, stream)
	}
	// For some requests, we don't have a sane mapping of incoming requests to a specific
	// downstream service when multiplexing. Only forward when we have only one backend.
	pub async fn send_single_without_multiplexing(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		let Some(service_name) = &self.upstreams.default_target_name else {
			return Err(UpstreamError::InvalidMethod(r.request.method().to_string()));
		};
		self.send_single(r, ctx, service_name).await
	}
	pub async fn send_fanout_deletion(
		&self,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		for (_, con) in self.upstreams.iter_named() {
			con.delete(&ctx).await?;
		}
		Ok(accepted_response())
	}
	pub async fn send_fanout_get(
		&self,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		let mut streams = Vec::new();
		for (name, con) in self.upstreams.iter_named() {
			streams.push((name, con.get_event_stream(&ctx).await?));
		}

		let ms = mergestream::MergeStream::new_without_merge(streams);
		messages_to_response(RequestId::Number(0), ms)
	}
	pub async fn send_fanout(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
	) -> Result<Response, UpstreamError> {
		let id = r.id.clone();
		let mut streams = Vec::new();
		for (name, con) in self.upstreams.iter_named() {
			streams.push((name, con.generic_stream(r.clone(), &ctx).await?));
		}

		let ms = mergestream::MergeStream::new(streams, id.clone(), merge);
		messages_to_response(id, ms)
	}
	pub async fn send_notification(
		&self,
		r: JsonRpcNotification<ClientNotification>,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		let mut streams = Vec::new();
		for (name, con) in self.upstreams.iter_named() {
			streams.push((
				name,
				con
					.generic_notification(r.notification.clone(), &ctx)
					.await?,
			));
		}

		Ok(accepted_response())
	}
	fn get_info(pv: ProtocolVersion, multiplexing: bool) -> ServerInfo {
		let capabilities = if multiplexing {
			ServerCapabilities {
				completions: None,
				experimental: None,
				logging: None,
				tasks: None,
				extensions: None,
				tools: Some(ToolsCapability::default()),
				// These are not supported when multiplexing.
				prompts: None,
				resources: None,
			}
		} else {
			ServerCapabilities {
				completions: None,
				experimental: None,
				logging: None,
				tasks: None,
				extensions: None,
				tools: Some(ToolsCapability::default()),
				prompts: Some(PromptsCapability::default()),
				resources: Some(ResourcesCapability::default()),
			}
		};
		let instructions = Some(
            "This server is a gateway to a set of mcp servers. It is responsible for routing requests to the correct server and aggregating the results.".to_string(),
        );
		ServerInfo {
			protocol_version: pv,
			capabilities,
			server_info: Implementation::from_build_env(),
			instructions,
		}
	}
}

pub fn setup_request_log(
	http: Parts,
	span_name: &str,
) -> (BoxedSpan, AsyncLog<MCPInfo>, CelExecWrapper) {
	let traceparent = http.extensions.get::<TraceParent>();
	let mut ctx = Context::new();
	if let Some(tp) = traceparent {
		ctx = ctx.with_remote_span_context(SpanContext::new(
			tp.trace_id.into(),
			tp.span_id.into(),
			TraceFlags::new(tp.flags),
			true,
			TraceState::default(),
		));
	}
	let claims = http.extensions.get::<Claims>().cloned();

	let log = http
		.extensions
		.get::<AsyncLog<MCPInfo>>()
		.cloned()
		.unwrap_or_default();

	let snap = http
		.extensions
		.get::<Arc<Option<RequestSnapshot>>>()
		.cloned()
		.unwrap_or_else(|| Arc::new(None));

	let cel = CelExecWrapper::new(snap);

	let tracer = trcng::get_tracer();
	let _span = trcng::start_span(span_name.to_string(), &Identity::new(claims))
		.with_kind(SpanKind::Server)
		.start_with_context(tracer, &ctx);
	(_span, log, cel)
}

fn messages_to_response(
	id: RequestId,
	stream: impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
) -> Result<Response, UpstreamError> {
	use futures_util::StreamExt;
	use rmcp::model::ServerJsonRpcMessage;
	let stream = stream.map(move |rpc| {
		let r = match rpc {
			Ok(rpc) => rpc,
			Err(e) => {
				ServerJsonRpcMessage::error(ErrorData::internal_error(e.to_string(), None), id.clone())
			},
		};
		// TODO: is it ok to have no event_id here?
		ServerSseMessage {
			event_id: None,
			message: Arc::new(r),
		}
	});
	Ok(mcp::session::sse_stream_response(stream, None))
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}
