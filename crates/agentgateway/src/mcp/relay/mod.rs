use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, LazyLock};

use crate::ProxyInputs;
use crate::cel::ContextBuilder;
use crate::http::Response;
use crate::http::jwt::Claims;
use crate::mcp::mergestream::MergeFn;
use crate::mcp::rbac::{Identity, McpAuthorizationSet};
use crate::mcp::relay::upstream::UpstreamError;
use crate::mcp::sse::{MCPInfo, McpBackendGroup};
use crate::mcp::{mergestream, rbac};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::AsyncLog;
use crate::telemetry::trc::TraceParent;
use crate::transport::stream::TLSConnectionInfo;
use agent_core::prelude::Strng;
use agent_core::trcng;
use agent_core::version::BuildInfo;
use http::StatusCode;
use http::request::Parts;
use itertools::Itertools;
use opentelemetry::global::BoxedSpan;
use opentelemetry::trace::{SpanContext, SpanKind, TraceContextExt, TraceState};
use opentelemetry::{Context, TraceFlags};
pub use pool::ClientError;
use rmcp::model::*;
use rmcp::service::{RequestContext, RunningService};
use rmcp::transport::common::server_side_http::ServerSseMessage;
use rmcp::{RoleClient, RoleServer, model};

type McpError = ErrorData;

pub mod metrics;
mod pool;
pub mod upstream;

const DELIMITER: &str = "_";

fn resource_name(default_target_name: Option<&String>, target: &str, name: &str) -> String {
	if default_target_name.is_none() {
		format!("{target}{DELIMITER}{name}")
	} else {
		name.to_string()
	}
}
static AGW_INITIALIZE: LazyLock<InitializeRequestParam> =
	LazyLock::new(|| InitializeRequestParam {
		protocol_version: ProtocolVersion::V_2025_03_26,
		capabilities: ClientCapabilities {
			// TODO(keithmattix): where do we document these?
			..Default::default()
		},
		client_info: Implementation {
			name: "agentgateway".to_string(),
			version: BuildInfo::new().version.to_string(),
		},
	});

#[derive(Clone, Debug)]
pub struct RqCtx {
	identity: Identity,
	context: Context,
}

impl Default for RqCtx {
	fn default() -> Self {
		Self {
			identity: Identity::default(),
			context: Context::new(),
		}
	}
}

impl RqCtx {
	pub fn new(identity: Identity, context: Context) -> Self {
		Self { identity, context }
	}
}

#[derive(Debug, Clone)]
pub struct Relay {
	pool: Arc<pool::ConnectionPool>,
	pub metrics: Arc<metrics::Metrics>,
	pub policies: McpAuthorizationSet,
	// If we have 1 target only, we don't prefix everything with 'target_'.
	// Else this is empty
	default_target_name: Option<String>,
	stateful: bool,
}

impl Relay {
	pub fn new(
		pi: Arc<ProxyInputs>,
		backend: McpBackendGroup,
		metrics: Arc<metrics::Metrics>,
		policies: McpAuthorizationSet,
		client: PolicyClient,
		stateful: bool,
	) -> anyhow::Result<Self> {
		let default_target_name = if backend.targets.len() != 1 {
			None
		} else {
			Some(backend.targets[0].name.to_string())
		};
		Ok(Self {
			pool: Arc::new(pool::ConnectionPool::new(pi, client, backend, stateful)?),
			metrics,
			policies,
			default_target_name,
			stateful,
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

	fn resource_name(&self, target: &str, name: &str) -> String {
		if self.default_target_name.is_none() {
			format!("{target}{DELIMITER}{name}")
		} else {
			name.to_string()
		}
	}

	fn setup_request(
		ext: &model::Extensions,
		span_name: &str,
	) -> Result<(BoxedSpan, RqCtx), McpError> {
		let (s, rq, _, _) = Self::setup_request_log(ext, span_name)?;
		Ok((s, rq))
	}
	fn setup_request_log(
		ext: &model::Extensions,
		span_name: &str,
	) -> Result<(BoxedSpan, RqCtx, AsyncLog<MCPInfo>, Arc<ContextBuilder>), McpError> {
		let Some(http) = ext.get::<Parts>() else {
			return Err(McpError::internal_error(
				"failed to extract parts".to_string(),
				None,
			));
		};
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
		let tls = http.extensions.get::<TLSConnectionInfo>();
		let id = tls
			.and_then(|tls| tls.src_identity.as_ref())
			.map(|src_id| src_id.to_string());

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

		let rq_ctx = RqCtx::new(Identity::new(claims.cloned(), id), ctx);

		let tracer = trcng::get_tracer();
		let _span = trcng::start_span(span_name.to_string(), &rq_ctx.identity)
			.with_kind(SpanKind::Server)
			.start_with_context(tracer, &rq_ctx.context);
		Ok((_span, rq_ctx, log, cel))
	}
}

impl Relay {
	pub async fn notify(&self, not: ClientNotification) -> Result<(), UpstreamError> {
		for con in self.pool.iter() {
			// TODO: For Progress and Cancel we need to route these to the correct destination!
			con.notify(not.clone()).await?;
		}
		Ok(())
	}
	// pub async fn initialize(&self, r: InitializeRequest) -> Result<InitializeResult, UpstreamError> {
	// 	// List servers and initialize the ones that are not initialized
	// 	// Initialize all targets
	// 	for con in self.pool.iter() {
	// 		let _res = con.initialize(r.params.clone()).await?;
	// 	}
	// 	// For now, return static info about ourselves
	// 	// In the future, merge the results from each upstream.
	// 	let res = self.get_info();
	// 	Ok(res)
	// }
	// pub async fn list_tools(
	// 	&self,
	// 	r: ListToolsRequest,
	// 	cel: &ContextBuilder,
	// ) -> Result<ListToolsResult, UpstreamError> {
	// 	let mut tools = Vec::new();
	// 	for (name, con) in self.pool.iter_named() {
	// 		let res = con.list_tools(r.params.clone()).await?;
	// 		res
	// 			.tools
	// 			.into_iter()
	// 			.filter(|t| {
	// 				self.policies.validate(
	// 					&rbac::ResourceType::Tool(rbac::ResourceId::new(name.to_string(), t.name.to_string())),
	// 					cel,
	// 				)
	// 			})
	// 			.for_each(|i| tools.push(i));
	// 	}
	//
	// 	self.metrics.clone().record(
	// 		metrics::ListCall {
	// 			resource_type: "tool".to_string(),
	// 			params: vec![],
	// 		},
	// 		(),
	// 	);
	//
	// 	Ok(ListToolsResult {
	// 		tools,
	// 		next_cursor: None,
	// 	})
	// }
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
						.filter(|t| {
							policies.validate(
								&rbac::ResourceType::Tool(rbac::ResourceId::new(
									server_name.to_string(),
									t.name.to_string(),
								)),
								&cel,
							)
						})
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
	pub fn merge_initialize(&self) -> Box<MergeFn> {
		let info = self.get_info();
		Box::new(move |streams| {
			// For now, we just send our own info. In the future, we should merge the results from each upstream.
			// TODO: set session info
			Ok(info.into())
		})
	}
	pub async fn send_single(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		user_headers: http::HeaderMap,
		service_name: &str,
	) -> Result<Response, UpstreamError> {
		let Ok(us) = self.pool.get(service_name) else {
			return Err(UpstreamError::InvalidRequest(format!(
				"unknown service {service_name}"
			)));
		};
		let stream = us.generic_stream(r, &user_headers).await?;

		messages_to_response(stream)
	}
	pub async fn send_fanout(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		user_headers: http::HeaderMap,
		merge: Box<MergeFn>,
	) -> Result<Response, UpstreamError> {
		let mut streams = Vec::new();
		for (name, con) in self.pool.iter_named() {
			streams.push((name, con.generic_stream(r.clone(), &user_headers).await?));
		}

		let ms = mergestream::MergeStream::new(streams, r.id, merge);
		merge_to_response(ms)
	}
	pub async fn send_notification(
		&self,
		r: JsonRpcNotification<ClientNotification>,
		user_headers: http::HeaderMap,
	) -> Result<Response, UpstreamError> {
		let mut streams = Vec::new();
		for (name, con) in self.pool.iter_named() {
			streams.push((
				name,
				con
					.generic_notification(r.notification.clone(), &user_headers)
					.await?,
			));
		}

		Ok(accepted_response())
	}
	// pub async fn list_tools2(
	// 	&self,
	// 	r: ListToolsRequest,
	// 	req_id: RequestId,
	// 	cel: Arc<ContextBuilder>,
	// ) -> Result<mergestream::MergeStream, UpstreamError> {
	// 	let mut streams = Vec::new();
	// 	for (name, con) in self.pool.iter_named() {
	// 		streams.push((name, con.list_tools2(r.params.clone()).await?));
	// 	}
	// 	let policies = self.policies.clone();
	// 	let default_target_name = self.default_target_name.clone();
	// 	Ok(mergestream::MergeStream::new(
	// 		streams,
	// 		req_id,
	// 		move |streams| {
	// 			let tools = streams
	// 				.into_iter()
	// 				.flat_map(|(server_name, s)| {
	// 					let tools = match s {
	// 						ServerResult::ListToolsResult(ltr) => ltr.tools,
	// 						_ => vec![],
	// 					};
	// 					tools
	// 						.into_iter()
	// 						.filter(|t| {
	// 							policies.validate(
	// 								&rbac::ResourceType::Tool(rbac::ResourceId::new(
	// 									server_name.to_string(),
	// 									t.name.to_string(),
	// 								)),
	// 								&cel,
	// 							)
	// 						})
	// 						.map(|t| Tool {
	// 							name: Cow::Owned(resource_name(
	// 								default_target_name.as_ref(),
	// 								server_name.as_str(),
	// 								&t.name,
	// 							)),
	// 							..t
	// 						})
	// 						.collect_vec()
	// 				})
	// 				.collect_vec();
	// 			Ok(
	// 				ListToolsResult {
	// 					tools,
	// 					next_cursor: None,
	// 				}
	// 				.into(),
	// 			)
	// 		},
	// 	))
	// }
	// pub async fn call_tool(
	// 	&self,
	// 	r: CallToolRequest,
	// 	cel: &ContextBuilder,
	// 	log: AsyncLog<MCPInfo>,
	// ) -> Result<CallToolResult, UpstreamError> {
	// 	let request = r.params;
	// 	let tool_name = request.name.to_string();
	// 	let (service_name, tool) = self.parse_resource_name(&tool_name)?;
	// 	log.non_atomic_mutate(|l| {
	// 		l.tool_call_name = Some(tool.to_string());
	// 		l.target_name = Some(service_name.to_string());
	// 	});
	// 	if !self.policies.validate(
	// 		&rbac::ResourceType::Tool(rbac::ResourceId::new(
	// 			service_name.to_string(),
	// 			tool.to_string(),
	// 		)),
	// 		cel,
	// 	) {
	// 		return Err(UpstreamError::Authorization);
	// 	}
	// 	let con = self.pool.get(service_name)?;
	// 	let req = CallToolRequestParam {
	// 		name: Cow::Owned(tool.to_string()),
	// 		arguments: request.arguments,
	// 	};
	// 	self.metrics.record(
	// 		metrics::ToolCall {
	// 			server: service_name.to_string(),
	// 			name: tool.to_string(),
	// 			params: vec![],
	// 		},
	// 		(),
	// 	);
	// 	con.call_tool(req).await;
	// 	// match con.call_tool(req).await {
	// 	// 	Ok(r) => Ok(r),
	// 	// 	Err(e) => {
	// 	// 		self.metrics.record(
	// 	// 			metrics::ToolCallError {
	// 	// 				server: service_name.to_string(),
	// 	// 				name: tool.to_string(),
	// 	// 				error_type: e.error_code(),
	// 	// 				params: vec![],
	// 	// 			},
	// 	// 			(),
	// 	// 		);
	// 	// 		Err(e.into())
	// 	// 	},
	// 	// };
	// 	todo!()
	// }

	fn get_info(&self) -> ServerInfo {
		ServerInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
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

// TODO: lists and gets can be macros
// impl ServerHandler for Relay {
// 	#[instrument(level = "debug", skip_all)]
// 	fn get_info(&self) -> ServerInfo {
// 		ServerInfo {
//             protocol_version: ProtocolVersion::V_2025_03_26,
//             capabilities: ServerCapabilities {
//                 completions: None,
//                 experimental: None,
//                 logging: None,
//                 prompts: Some(PromptsCapability::default()),
//                 resources: Some(ResourcesCapability::default()),
//                 tools: Some(ToolsCapability::default()),
//             },
//             server_info: Implementation::from_build_env(),
//             instructions: Some(
//                 "This server is a gateway to a set of mcp servers. It is responsible for routing requests to the correct server and aggregating the results.".to_string(),
//             ),
//         }
// 	}
//
// 	// The client will send an initialize request with their parameters. We will return our own static support
// 	async fn initialize(
// 		&self,
// 		request: InitializeRequestParam,
// 		context: RequestContext<RoleServer>,
// 	) -> Result<InitializeResult, McpError> {
// 		let (_span, _) = Self::setup_request(&context.extensions, "initialize")?;
//
// 		// List servers and initialize the ones that are not initialized
// 		let mut pool = self.pool.write().await;
// 		// Initialize all targets
// 		let _ = pool
// 			.initialize(&context.peer, request)
// 			.await
// 			.map_err(|e| McpError::internal_error(format!("Failed to list connections: {e}"), None))?;
//
// 		// Return static server info about ourselves
// 		// TODO: we should actually perform an intersection of what the downstream and we support. The problem
// 		// is we may connect to many upstream servers, how do expose what exactly we can and cannot support?
// 		Ok(self.get_info())
// 	}
//
// 	#[instrument(level = "debug", skip_all)]
// 	async fn list_resources(
// 		&self,
// 		request: Option<PaginatedRequestParam>,
// 		context: RequestContext<RoleServer>,
// 	) -> std::result::Result<ListResourcesResult, McpError> {
// 		let (_span, ref rq_ctx) = Self::setup_request(&context.extensions, "list_resources")?;
// 		let mut pool = self.pool.write().await;
// 		let connections = self.list_conns(&context, pool.deref_mut()).await?;
// 		let all = connections.into_iter().map(|(_name, svc)| {
// 			let request = request.clone();
// 			async move {
// 				match svc.list_resources(request, rq_ctx).await {
// 					Ok(r) => Ok(r.resources),
// 					Err(e) => Err(e),
// 				}
// 			}
// 		});
//
// 		// TODO: Handle errors
// 		let (results, _errors): (Vec<_>, Vec<_>) = futures::future::join_all(all)
// 			.await
// 			.into_iter()
// 			.partition_result();
//
// 		Ok(ListResourcesResult {
// 			resources: results.into_iter().flatten().collect(),
// 			next_cursor: None,
// 		})
// 	}
//
// 	#[instrument(level = "debug", skip_all)]
// 	async fn list_resource_templates(
// 		&self,
// 		request: Option<PaginatedRequestParam>,
// 		context: RequestContext<RoleServer>,
// 	) -> std::result::Result<ListResourceTemplatesResult, McpError> {
// 		let (_span, ref rq_ctx) = Self::setup_request(&context.extensions, "list_resource_templates")?;
//
// 		let mut pool = self.pool.write().await;
// 		let connections = self.list_conns(&context, pool.deref_mut()).await?;
// 		let all = connections.into_iter().map(|(_name, svc)| {
// 			let request = request.clone();
// 			async move {
// 				match svc.list_resource_templates(request, rq_ctx).await {
// 					Ok(r) => Ok(r.resource_templates),
// 					Err(e) => Err(e),
// 				}
// 			}
// 		});
//
// 		let (results, _errors): (Vec<_>, Vec<_>) = futures::future::join_all(all)
// 			.await
// 			.into_iter()
// 			.partition_result();
//
// 		self.metrics.clone().record(
// 			metrics::ListCall {
// 				resource_type: "resource_template".to_string(),
// 				params: vec![],
// 			},
// 			(),
// 		);
//
// 		Ok(ListResourceTemplatesResult {
// 			resource_templates: results.into_iter().flatten().collect(),
// 			next_cursor: None,
// 		})
// 	}
//
// 	#[instrument(level = "debug", skip_all)]
// 	async fn list_prompts(
// 		&self,
// 		request: Option<PaginatedRequestParam>,
// 		context: RequestContext<RoleServer>,
// 	) -> std::result::Result<ListPromptsResult, McpError> {
// 		let (_span, ref rq_ctx) = Self::setup_request(&context.extensions, "list_prompts")?;
//
// 		let mut pool = self.pool.write().await;
// 		let connections = self.list_conns(&context, pool.deref_mut()).await?;
//
// 		let all = connections.into_iter().map(|(_name, svc)| {
// 			let request = request.clone();
// 			async move {
// 				match svc.list_prompts(request, rq_ctx).await {
// 					Ok(r) => Ok(
// 						r.prompts
// 							.into_iter()
// 							.map(|p| Prompt {
// 								name: self.resource_name(_name.as_str(), &p.name),
// 								description: p.description,
// 								arguments: p.arguments,
// 							})
// 							.collect::<Vec<_>>(),
// 					),
// 					Err(e) => Err(e),
// 				}
// 			}
// 		});
//
// 		let (results, _errors): (Vec<_>, Vec<_>) = futures::future::join_all(all)
// 			.await
// 			.into_iter()
// 			.partition_result();
//
// 		self.metrics.record(
// 			metrics::ListCall {
// 				resource_type: "prompt".to_string(),
// 				params: vec![],
// 			},
// 			(),
// 		);
// 		Ok(ListPromptsResult {
// 			prompts: results.into_iter().flatten().collect(),
// 			next_cursor: None,
// 		})
// 	}
//
// 	#[instrument(
//         level = "debug",
//         skip_all,
//         fields(
//         name=%request.uri,
//         ),
//     )]
// 	async fn read_resource(
// 		&self,
// 		request: ReadResourceRequestParam,
// 		context: RequestContext<RoleServer>,
// 	) -> std::result::Result<ReadResourceResult, McpError> {
// 		let (_span, ref rq_ctx, _, cel) =
// 			Self::setup_request_log(&context.extensions, "read_resource")?;
//
// 		let uri = request.uri.to_string();
// 		let (service_name, resource) = self.parse_resource_name(&uri)?;
// 		if !self.policies.validate(
// 			&rbac::ResourceType::Resource(rbac::ResourceId::new(
// 				service_name.to_string(),
// 				resource.to_string(),
// 			)),
// 			cel.as_ref(),
// 		) {
// 			return Err(McpError::invalid_request("not allowed", None));
// 		}
// 		let req = ReadResourceRequestParam {
// 			uri: resource.to_string(),
// 		};
// 		let mut pool = self.pool.write().await;
// 		let service = self
// 			.get_conn(&context, rq_ctx, pool.deref_mut(), service_name)
// 			.await?;
//
// 		self.metrics.clone().record(
// 			metrics::GetResourceCall {
// 				server: service_name.to_string(),
// 				uri: resource.to_string(),
// 				params: vec![],
// 			},
// 			(),
// 		);
// 		match service.read_resource(req, rq_ctx).await {
// 			Ok(r) => Ok(r),
// 			Err(e) => Err(e.into()),
// 		}
// 	}
//
// 	#[instrument(
//         level = "debug",
//         skip_all,
//         fields(
//         name=%request.name,
//         ),
//     )]
// 	async fn get_prompt(
// 		&self,
// 		request: GetPromptRequestParam,
// 		context: RequestContext<RoleServer>,
// 	) -> std::result::Result<GetPromptResult, McpError> {
// 		let (_span, ref rq_ctx, _, cel) = Self::setup_request_log(&context.extensions, "get_prompt")?;
//
// 		let prompt_name = request.name.to_string();
// 		let (service_name, prompt) = self.parse_resource_name(&prompt_name)?;
// 		if !self.policies.validate(
// 			&rbac::ResourceType::Prompt(rbac::ResourceId::new(
// 				service_name.to_string(),
// 				prompt.to_string(),
// 			)),
// 			cel.as_ref(),
// 		) {
// 			return Err(McpError::invalid_request("not allowed", None));
// 		}
// 		let mut pool = self.pool.write().await;
// 		let req = GetPromptRequestParam {
// 			name: prompt.to_string(),
// 			arguments: request.arguments,
// 		};
// 		let svc = self
// 			.get_conn(&context, rq_ctx, pool.deref_mut(), service_name)
// 			.await?;
//
// 		self.metrics.clone().record(
// 			metrics::GetPromptCall {
// 				server: service_name.to_string(),
// 				name: prompt.to_string(),
// 				params: vec![],
// 			},
// 			(),
// 		);
// 		match svc.get_prompt(req, rq_ctx).await {
// 			Ok(r) => Ok(r),
// 			Err(e) => Err(e.into()),
// 		}
// 	}
//
// 	#[instrument(level = "debug", skip_all)]
// 	async fn list_tools(
// 		&self,
// 		request: Option<PaginatedRequestParam>,
// 		context: RequestContext<RoleServer>,
// 	) -> std::result::Result<ListToolsResult, McpError> {
// 		let (_span, ref rq_ctx, _, cel) = Self::setup_request_log(&context.extensions, "list_tools")?;
// 		let mut pool = self.pool.write().await;
// 		let connections = self.list_conns(&context, pool.deref_mut()).await?;
// 		let all = connections.into_iter().map(|(_name, svc_arc)| {
// 			let request = request.clone();
// 			let cel = cel.clone();
// 			async move {
// 				match svc_arc.list_tools(request, rq_ctx).await {
// 					Ok(r) => Ok(
// 						r.tools
// 							.into_iter()
// 							.filter(|t| {
// 								self.policies.validate(
// 									&rbac::ResourceType::Tool(rbac::ResourceId::new(
// 										_name.to_string(),
// 										t.name.to_string(),
// 									)),
// 									cel.as_ref(),
// 								)
// 							})
// 							.map(|t| Tool {
// 								annotations: None,
// 								name: Cow::Owned(self.resource_name(_name.as_str(), &t.name)),
// 								..t
// 							})
// 							.collect::<Vec<_>>(),
// 					),
// 					Err(e) => Err(e),
// 				}
// 			}
// 		});
//
// 		let (results, _errors): (Vec<_>, Vec<_>) = futures::future::join_all(all)
// 			.await
// 			.into_iter()
// 			.partition_result();
//
// 		self.metrics.clone().record(
// 			metrics::ListCall {
// 				resource_type: "tool".to_string(),
// 				params: vec![],
// 			},
// 			(),
// 		);
//
// 		Ok(ListToolsResult {
// 			tools: results.into_iter().flatten().collect(),
// 			next_cursor: None,
// 		})
// 	}
//
// 	#[instrument(
//         level = "debug",
//         skip_all,
//         fields(
//         name=%request.name,
//         ),
//     )]
// 	fn call_tool(
// 		&self,
// 		request: CallToolRequestParam,
// 		context: RequestContext<RoleServer>,
// 	) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
// 		Box::pin(async move {
// 			let (_span, ref rq_ctx, log, cel) =
// 				Self::setup_request_log(&context.extensions, "call_tool")?;
// 			let tool_name = request.name.to_string();
// 			let (service_name, tool) = self.parse_resource_name(&tool_name)?;
// 			log.non_atomic_mutate(|l| {
// 				l.tool_call_name = Some(tool.to_string());
// 				l.target_name = Some(service_name.to_string());
// 			});
// 			if !self.policies.validate(
// 				&rbac::ResourceType::Tool(rbac::ResourceId::new(
// 					service_name.to_string(),
// 					tool.to_string(),
// 				)),
// 				cel.as_ref(),
// 			) {
// 				return Err(McpError::invalid_request("not allowed", None));
// 			}
// 			let mut pool = self.pool.write().await;
// 			let req = CallToolRequestParam {
// 				name: Cow::Owned(tool.to_string()),
// 				arguments: request.arguments,
// 			};
// 			let svc = self
// 				.get_conn(&context, rq_ctx, pool.deref_mut(), service_name)
// 				.await?;
// 			self.metrics.record(
// 				metrics::ToolCall {
// 					server: service_name.to_string(),
// 					name: tool.to_string(),
// 					params: vec![],
// 				},
// 				(),
// 			);
// 			match svc.call_tool(req, rq_ctx).await {
// 				Ok(r) => Ok(r),
// 				Err(e) => {
// 					self.metrics.record(
// 						metrics::ToolCallError {
// 							server: service_name.to_string(),
// 							name: tool.to_string(),
// 							error_type: e.error_code(),
// 							params: vec![],
// 						},
// 						(),
// 					);
// 					Err(e.into())
// 				},
// 			}
// 		})
// 	}
// }

pub fn setup_request_log2(
	http: &Parts,
	span_name: &str,
) -> (BoxedSpan, RqCtx, AsyncLog<MCPInfo>, Arc<ContextBuilder>) {
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
	let tls = http.extensions.get::<TLSConnectionInfo>();
	let id = tls
		.and_then(|tls| tls.src_identity.as_ref())
		.map(|src_id| src_id.to_string());

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

	let rq_ctx = RqCtx::new(Identity::new(claims.cloned(), id), ctx);

	let tracer = trcng::get_tracer();
	let _span = trcng::start_span(span_name.to_string(), &rq_ctx.identity)
		.with_kind(SpanKind::Server)
		.start_with_context(tracer, &rq_ctx.context);
	(_span, rq_ctx, log, cel)
}

fn merge_to_response(stream: super::mergestream::MergeStream) -> Result<Response, UpstreamError> {
	use futures_util::StreamExt;
	let stream = stream.map(|rpc| {
		let r = match rpc {
			Ok(rpc) => rpc,
			// TODO: do not hardcode number
			Err(e) => ServerJsonRpcMessage::error(
				ErrorData::internal_error(e.to_string(), None),
				RequestId::Number(2),
			),
		};
		// TODO: is it ok to have no event_id here?
		ServerSseMessage {
			event_id: None,
			message: Arc::new(r),
		}
	});
	Ok(crate::mcp::streamablehttp::sse_stream_response(
		stream, None,
	))
}
fn messages_to_response(stream: super::mergestream::Messages) -> Result<Response, UpstreamError> {
	use futures_util::StreamExt;
	let stream = stream.map(|rpc| {
		let r = match rpc {
			Ok(rpc) => rpc,
			// TODO: do not hardcode number
			Err(e) => ServerJsonRpcMessage::error(
				ErrorData::internal_error(e.to_string(), None),
				RequestId::Number(2),
			),
		};
		// TODO: is it ok to have no event_id here?
		ServerSseMessage {
			event_id: None,
			message: Arc::new(r),
		}
	});
	Ok(crate::mcp::streamablehttp::sse_stream_response(
		stream, None,
	))
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}
