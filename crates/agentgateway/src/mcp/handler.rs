use agent_core::strng::Strng;
use agent_core::trcng;
use futures_core::Stream;
use futures_util::StreamExt;
use http::StatusCode;
use http::request::Parts;
use itertools::Itertools;
use opentelemetry::global::BoxedSpan;
use opentelemetry::trace::{SpanContext, SpanKind, TraceContextExt, TraceState};
use opentelemetry::{Context, TraceFlags};
use rmcp::ErrorData;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, Implementation, JsonRpcNotification,
	JsonRpcRequest, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
	ListTasksResult, ListToolsResult, Meta, Prompt, PromptsCapability, ProtocolVersion, RequestId,
	ResourcesCapability, ServerCapabilities, ServerInfo, ServerJsonRpcMessage, ServerResult, Tool,
	ToolsCapability,
};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::sync::Arc;

use crate::cel::ContextBuilder;
use crate::http::Response;
use crate::http::jwt::Claims;
use crate::http::sessionpersistence::MCPSession;
use crate::mcp::elicitation;
use crate::mcp::mergestream::{MergeFn, MessageMapper};
use crate::mcp::rbac::{Identity, McpAuthorizationSet};
use crate::mcp::router::McpBackendGroup;
use crate::mcp::streamablehttp::ServerSseMessage;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, MCPInfo, mergestream, rbac, upstream};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::AsyncLog;
use crate::telemetry::trc::TraceParent;

const DELIMITER: &str = "_";
const UPSTREAM_REQUEST_ID_PREFIX: &str = "agw";
const UPSTREAM_REQUEST_ID_SEPARATOR: &str = "::";
const UPSTREAM_REQUEST_ID_KIND_SEPARATOR: &str = ":";

fn resource_name(default_target_name: Option<&String>, target: &str, name: &str) -> String {
	if default_target_name.is_none() {
		format!("{target}{DELIMITER}{name}")
	} else {
		name.to_string()
	}
}

fn merge_meta(entries: impl IntoIterator<Item = (Strng, Option<Meta>)>) -> Option<Meta> {
	let mut items = Vec::new();
	for (server_name, meta) in entries {
		if let Some(meta) = meta {
			items.push((server_name, meta));
		}
	}
	match items.len() {
		0 => None,
		1 => Some(items.pop().expect("len checked").1),
		_ => {
			let mut per_upstream = Map::new();
			for (server_name, meta) in items {
				per_upstream.insert(server_name.to_string(), Value::Object(meta.0));
			}
			let mut root = Map::new();
			root.insert("upstreams".to_string(), Value::Object(per_upstream));
			Some(Meta(root))
		},
	}
}

#[derive(Debug, Clone)]
pub struct Relay {
	upstreams: Arc<upstream::UpstreamGroup>,
	pub policies: McpAuthorizationSet,
	// If we have 1 target only, we don't prefix everything with 'target_'.
	// Else this is empty
	default_target_name: Option<String>,
	is_multiplexing: bool,
}

impl Relay {
	pub fn new(
		backend: McpBackendGroup,
		policies: McpAuthorizationSet,
		client: PolicyClient,
	) -> anyhow::Result<Self> {
		let mut is_multiplexing = false;
		let default_target_name = if backend.targets.len() != 1 {
			is_multiplexing = true;
			None
		} else if backend.targets[0].always_use_prefix {
			None
		} else {
			Some(backend.targets[0].name.to_string())
		};
		Ok(Self {
			upstreams: Arc::new(upstream::UpstreamGroup::new(client, backend)?),
			policies,
			default_target_name,
			is_multiplexing,
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

	fn should_prefix_identifiers(&self) -> bool {
		self.default_target_name.is_none()
	}

	fn encode_upstream_request_id(&self, server_name: &str, id: &RequestId) -> RequestId {
		if !self.should_prefix_identifiers() {
			return id.clone();
		}
		let (kind, value) = match id {
			RequestId::Number(n) => ("n", n.to_string()),
			RequestId::String(s) => ("s", s.to_string()),
		};
		RequestId::String(
			format!(
				"{UPSTREAM_REQUEST_ID_PREFIX}{UPSTREAM_REQUEST_ID_SEPARATOR}{server_name}{UPSTREAM_REQUEST_ID_SEPARATOR}{kind}{UPSTREAM_REQUEST_ID_KIND_SEPARATOR}{value}"
			)
			.into(),
		)
	}

	pub fn decode_upstream_request_id(
		&self,
		id: &RequestId,
	) -> Result<(String, RequestId), UpstreamError> {
		if let Some(default) = self.default_target_name.as_deref() {
			return Ok((default.to_string(), id.clone()));
		}
		let RequestId::String(raw) = id else {
			return Err(UpstreamError::InvalidRequest(
				"upstream request id must be a string when multiplexing".to_string(),
			));
		};
		let raw = raw.as_ref();
		let Some((prefix, rest)) = raw.split_once(UPSTREAM_REQUEST_ID_SEPARATOR) else {
			return Err(UpstreamError::InvalidRequest(
				"upstream request id missing gateway prefix".to_string(),
			));
		};
		if prefix != UPSTREAM_REQUEST_ID_PREFIX {
			return Err(UpstreamError::InvalidRequest(
				"upstream request id missing gateway prefix".to_string(),
			));
		}
		let Some((server_name, rest)) = rest.split_once(UPSTREAM_REQUEST_ID_SEPARATOR) else {
			return Err(UpstreamError::InvalidRequest(
				"upstream request id missing server name".to_string(),
			));
		};
		let Some((kind, value)) = rest.split_once(UPSTREAM_REQUEST_ID_KIND_SEPARATOR) else {
			return Err(UpstreamError::InvalidRequest(
				"upstream request id missing kind".to_string(),
			));
		};
		let orig_id = match kind {
			"n" => value
				.parse::<i64>()
				.ok()
				.map(RequestId::Number)
				.ok_or_else(|| {
					UpstreamError::InvalidRequest("upstream request id number parse failed".to_string())
				})?,
			"s" => RequestId::String(value.into()),
			_ => {
				return Err(UpstreamError::InvalidRequest(
					"upstream request id kind unknown".to_string(),
				));
			},
		};
		Ok((server_name.to_string(), orig_id))
	}

	fn map_server_message(
		&self,
		server_name: &str,
		mut message: ServerJsonRpcMessage,
	) -> ServerJsonRpcMessage {
		match &mut message {
			ServerJsonRpcMessage::Request(req) => {
				req.id = self.encode_upstream_request_id(server_name, &req.id);
				if let Some(params) = elicitation::extract_url_elicitation(&req.request) {
					tracing::debug!(
						elicitation_id = %params.elicitation_id,
						"received url elicitation request"
					);
				}
			},
			ServerJsonRpcMessage::Response(resp) => {
				self.map_server_result(server_name, &mut resp.result);
			},
			_ => {},
		}
		message
	}

	fn map_server_result(&self, server_name: &str, result: &mut ServerResult) {
		if !self.should_prefix_identifiers() {
			return;
		}
		match result {
			ServerResult::CreateTaskResult(r) => {
				r.task.task_id = resource_name(
					self.default_target_name.as_ref(),
					server_name,
					&r.task.task_id,
				);
			},
			ServerResult::ListTasksResult(r) => {
				for task in &mut r.tasks {
					task.task_id = resource_name(
						self.default_target_name.as_ref(),
						server_name,
						&task.task_id,
					);
				}
			},
			ServerResult::GetTaskInfoResult(r) => {
				if let Some(task) = &mut r.task {
					task.task_id = resource_name(
						self.default_target_name.as_ref(),
						server_name,
						&task.task_id,
					);
				}
			},
			_ => {},
		}
	}
}

impl Relay {
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
		self.is_multiplexing
	}
	pub fn default_target_name(&self) -> Option<String> {
		self.default_target_name.clone()
	}

	pub fn merge_tools(&self, cel: Arc<ContextBuilder>) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.default_target_name.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::new();
			let tools = streams
				.into_iter()
				.flat_map(|(server_name, s)| {
					let (tools, meta) = match s {
						ServerResult::ListToolsResult(ltr) => (ltr.tools, ltr.meta),
						_ => (vec![], None),
					};
					meta_entries.push((server_name.clone(), meta));
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
			let meta = merge_meta(meta_entries);
			Ok(
				ListToolsResult {
					tools,
					next_cursor: None,
					meta,
				}
				.into(),
			)
		})
	}

	pub fn merge_initialize(&self, pv: ProtocolVersion) -> Box<MergeFn> {
		Box::new(move |s| {
			if s.len() == 1 {
				let (_, ServerResult::InitializeResult(ir)) = s.into_iter().next().unwrap() else {
					return Ok(Self::get_info(pv).into());
				};
				return Ok(ir.clone().into());
			}

			let lowest_version = s
				.into_iter()
				.flat_map(|(_, v)| match v {
					ServerResult::InitializeResult(r) => Some(r.protocol_version),
					_ => None,
				})
				.min_by_key(|i| i.to_string())
				.unwrap_or(pv);
			// For now, we just send our own info. In the future, we should merge the results from each upstream.
			Ok(Self::get_info(lowest_version).into())
		})
	}

	pub fn merge_prompts(&self, cel: Arc<ContextBuilder>) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.default_target_name.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::new();
			let prompts = streams
				.into_iter()
				.flat_map(|(server_name, s)| {
					let (prompts, meta) = match s {
						ServerResult::ListPromptsResult(lpr) => (lpr.prompts, lpr.meta),
						_ => (vec![], None),
					};
					meta_entries.push((server_name.clone(), meta));
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
			let meta = merge_meta(meta_entries);
			Ok(
				ListPromptsResult {
					prompts,
					next_cursor: None,
					meta,
				}
				.into(),
			)
		})
	}
	pub fn merge_resources(&self, cel: Arc<ContextBuilder>) -> Box<MergeFn> {
		let policies = self.policies.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::new();
			let resources = streams
				.into_iter()
				.flat_map(|(server_name, s)| {
					let (resources, meta) = match s {
						ServerResult::ListResourcesResult(lrr) => (lrr.resources, lrr.meta),
						_ => (vec![], None),
					};
					meta_entries.push((server_name.clone(), meta));
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
			let meta = merge_meta(meta_entries);
			Ok(
				ListResourcesResult {
					resources,
					next_cursor: None,
					meta,
				}
				.into(),
			)
		})
	}
	pub fn merge_resource_templates(&self, cel: Arc<ContextBuilder>) -> Box<MergeFn> {
		let policies = self.policies.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::new();
			let resource_templates = streams
				.into_iter()
				.flat_map(|(server_name, s)| {
					let (resource_templates, meta) = match s {
						ServerResult::ListResourceTemplatesResult(lrr) => (lrr.resource_templates, lrr.meta),
						_ => (vec![], None),
					};
					meta_entries.push((server_name.clone(), meta));
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
			let meta = merge_meta(meta_entries);
			Ok(
				ListResourceTemplatesResult {
					resource_templates,
					next_cursor: None,
					meta,
				}
				.into(),
			)
		})
	}
	pub fn merge_tasks(&self) -> Box<MergeFn> {
		let default_target_name = self.default_target_name.clone();
		Box::new(move |streams| {
			let tasks = streams
				.into_iter()
				.flat_map(|(server_name, s)| {
					let tasks = match s {
						ServerResult::ListTasksResult(ltr) => ltr.tasks,
						_ => vec![],
					};
					tasks
						.into_iter()
						.map(|mut task| {
							task.task_id = resource_name(
								default_target_name.as_ref(),
								server_name.as_str(),
								&task.task_id,
							);
							task
						})
						.collect_vec()
				})
				.collect_vec();
			Ok(
				ListTasksResult {
					tasks,
					next_cursor: None,
					total: None,
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
		let relay = self.clone();
		let server_name = service_name.to_string();
		let stream = us
			.generic_stream(r, &ctx)
			.await?
			.map(move |msg| msg.map(|msg| relay.map_server_message(&server_name, msg)));

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

		let message_mapper: Option<MessageMapper> = if self.should_prefix_identifiers() {
			let relay = self.clone();
			Some(Arc::new(move |server_name: &str, message| {
				relay.map_server_message(server_name, message)
			}))
		} else {
			None
		};
		let ms = mergestream::MergeStream::new_without_merge(streams, message_mapper);
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

		let message_mapper: Option<MessageMapper> = if self.should_prefix_identifiers() {
			let relay = self.clone();
			Some(Arc::new(move |server_name: &str, message| {
				relay.map_server_message(server_name, message)
			}))
		} else {
			None
		};
		let ms = mergestream::MergeStream::new(streams, id.clone(), merge, message_mapper);
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
	pub async fn send_client_message(
		&self,
		service_name: String,
		message: ClientJsonRpcMessage,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		let Ok(us) = self.upstreams.get(&service_name) else {
			return Err(UpstreamError::InvalidRequest(format!(
				"unknown service {service_name}"
			)));
		};
		us.send_client_message(message, &ctx).await?;
		Ok(accepted_response())
	}
	fn get_info(pv: ProtocolVersion) -> ServerInfo {
		ServerInfo {
            protocol_version: pv,
            capabilities: ServerCapabilities {
                completions: None,
                experimental: None,
                logging: None,
                prompts: Some(PromptsCapability::default()),
                resources: Some(ResourcesCapability::default()),
                tools: Some(ToolsCapability::default()),
                tasks: None,
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

#[cfg(test)]
mod tests {
	use super::*;
	use agent_core::strng;
	use serde_json::json;

	#[test]
	fn merge_meta_includes_upstreams() {
		let mut meta_a = Meta::new();
		meta_a.0.insert("a".to_string(), json!(1));
		let mut meta_b = Meta::new();
		meta_b.0.insert("b".to_string(), json!(2));
		let merged = merge_meta(vec![
			(strng::new("a"), Some(meta_a)),
			(strng::new("b"), Some(meta_b)),
		])
		.expect("merged meta");
		let upstreams = merged
			.0
			.get("upstreams")
			.and_then(|v| v.as_object())
			.expect("meta.upstreams");
		assert!(upstreams.contains_key("a"));
		assert!(upstreams.contains_key("b"));
	}
}
