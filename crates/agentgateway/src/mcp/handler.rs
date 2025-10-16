use std::borrow::Cow;
use std::sync::Arc;

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

use crate::ProxyInputs;
use crate::cel::ContextBuilder;
use crate::http::Response;
use crate::http::jwt::Claims;
use crate::mcp::mergestream::MergeFn;
use crate::mcp::rbac::{Identity, McpAuthorizationSet};
use crate::mcp::router::McpBackendGroup;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, MCPInfo, mergestream, rbac, upstream};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::AsyncLog;
use crate::telemetry::trc::TraceParent;

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
	// If we have 1 target only, we don't prefix everything with 'target_'.
	// Else this is empty
	default_target_name: Option<String>,
}

impl Relay {
	pub fn new(
		pi: Arc<ProxyInputs>,
		backend: McpBackendGroup,
		policies: McpAuthorizationSet,
		client: PolicyClient,
	) -> anyhow::Result<Self> {
		let default_target_name = if backend.targets.len() != 1 {
			None
		} else {
			Some(backend.targets[0].name.to_string())
		};
		Ok(Self {
			upstreams: Arc::new(upstream::UpstreamGroup::new(pi, client, backend)?),
			policies,
			default_target_name,
		})
	}

	pub fn parse_resource_name<'a, 'b: 'a>(
		&'a self,
		res: &'b str,
	) -> Result<(&'a str, &'b str), UpstreamError> {
		if let Some(default) = self.default_target_name.as_ref() {
			Ok((default.as_str(), res))
		} else {
			res
				.split_once(DELIMITER)
				.ok_or(UpstreamError::InvalidRequest(
					"invalid resource name".to_string(),
				))
		}
	}
}

impl Relay {
	pub fn is_multiplexing(&self) -> bool {
		self.default_target_name.is_none()
	}
	pub fn default_target_name(&self) -> Option<String> {
		self.default_target_name.clone()
	}

	pub fn merge_tools(&self, cel: Arc<ContextBuilder>) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.default_target_name.clone();
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
				}
				.into(),
			)
		})
	}

	pub fn merge_initialize(&self, pv: ProtocolVersion) -> Box<MergeFn> {
		let info = self.get_info(pv);
		Box::new(move |_| {
			// For now, we just send our own info. In the future, we should merge the results from each upstream.
			Ok(info.into())
		})
	}

	pub fn merge_prompts(&self, cel: Arc<ContextBuilder>) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.default_target_name.clone();
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
				}
				.into(),
			)
		})
	}
	pub fn merge_resources(&self, cel: Arc<ContextBuilder>) -> Box<MergeFn> {
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
				}
				.into(),
			)
		})
	}
	pub fn merge_resource_templates(&self, cel: Arc<ContextBuilder>) -> Box<MergeFn> {
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
		let Some(service_name) = &self.default_target_name else {
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
	fn get_info(&self, pv: ProtocolVersion) -> ServerInfo {
		ServerInfo {
			protocol_version: pv,
			capabilities: ServerCapabilities {
				completions: None,
				experimental: None,
				logging: None,
				prompts: Some(PromptsCapability::default()),
				resources: Some(ResourcesCapability::default()),
				tools: Some(ToolsCapability::default()),
			},
			server_info: Implementation::from_build_env(),
			instructions: Some(
				"This server is a gateway to a set of mcp servers. It is responsible for routing requests to the correct server and aggregating the results.".to_string(),
			),
		}
	}
}

pub fn setup_request_log(
	http: &Parts,
	span_name: &str,
) -> (BoxedSpan, AsyncLog<MCPInfo>, Arc<ContextBuilder>) {
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
	let claims = http.extensions.get::<Claims>();

	let log = http
		.extensions
		.get::<AsyncLog<MCPInfo>>()
		.cloned()
		.unwrap_or_default();

	let cel = http
		.extensions
		.get::<Arc<ContextBuilder>>()
		.cloned()
		.expect("CelContextBuilder must be set");

	let tracer = trcng::get_tracer();
	let _span = trcng::start_span(span_name.to_string(), &Identity::new(claims.cloned()))
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
	use rmcp::transport::common::server_side_http::ServerSseMessage;
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
	Ok(crate::mcp::session::sse_stream_response(stream, None))
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}
