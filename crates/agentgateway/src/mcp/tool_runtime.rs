//! A gateway-initiated MCP client for one target of a configured MCP backend.
//!
//! The MCP proxy relays a downstream client's session. This runtime lets another part of the
//! gateway call a tool on the same backends, with the same backend policies, authorization rules
//! and guardrails, on behalf of a request that is not itself an MCP session.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use ::http::request::Parts;
use agent_llm::server_tools::ToolDefinition;
use rmcp::model::{ClientRequest, JsonRpcRequest, RequestId, ServerResult};
use serde_json::Value;

use crate::http::authorization::RuleSets;
use crate::mcp::handler::Relay;
use crate::mcp::router::{McpBackendGroup, McpTarget};
use crate::mcp::upstream::{IncomingRequestContext, Upstream};
use crate::mcp::{MCPInfo, McpAuthorizationSet, rbac};
use crate::telemetry::log::AsyncLog;
use crate::types::agent::{Backend, BackendKey, BackendTargetRef};
use crate::*;

// `UpstreamError` carries a response body and is not `Sync`; keep its message.
fn relay_error(e: crate::mcp::upstream::UpstreamError) -> anyhow::Error {
	anyhow::anyhow!("{e}")
}

/// The result of one tool call.
pub(crate) struct CallOutcome {
	/// MCP content items, as JSON.
	pub content: Vec<Value>,
	pub is_error: bool,
}

pub(crate) struct ToolRuntime {
	relay: Relay,
	target: Strng,
	ctx: IncomingRequestContext,
	initialized: tokio::sync::OnceCell<()>,
	next_id: AtomicI64,
}

impl ToolRuntime {
	/// Resolve `backend` (an MCP backend) from the store and prepare a client for `target`, or
	/// for its only target when `target` is `None`. `parts` supplies the caller's headers and
	/// identity for the MCP calls.
	pub(crate) fn new(
		inputs: &Arc<ProxyInputs>,
		backend_key: &BackendKey,
		target: Option<&str>,
		parts: &Parts,
	) -> anyhow::Result<Self> {
		let (name, backend, backend_policies) = {
			let binds = inputs.stores.read_binds();
			let be = binds
				.backend(backend_key)
				.ok_or_else(|| anyhow::anyhow!("MCP backend {backend_key} not found"))?;
			let Backend::MCP(name, backend) = &be.backend else {
				anyhow::bail!("backend {backend_key} is not an MCP backend");
			};
			let inline = be.inline_policies.clone();
			let policies = binds.backend_policies(
				BackendTargetRef::Backend {
					name: name.name.as_ref(),
					namespace: name.namespace.as_ref(),
					section: None,
				},
				&[inline.as_slice()],
				None,
			);
			(name.clone(), backend.clone(), policies)
		};
		let mut candidates = backend
			.targets
			.iter()
			.filter(|t| target.is_none_or(|want| t.name == want));
		let Some(selected) = candidates.next() else {
			anyhow::bail!(
				"MCP backend {backend_key} has no target {}",
				target.unwrap_or_default()
			);
		};
		if candidates.next().is_some() {
			anyhow::bail!("MCP backend {backend_key} has multiple targets; set the target explicitly");
		}
		let resolved = selected
			.spec
			.backend()
			.map(|b| crate::proxy::resolve_simple_backend_with_policies(b, inputs))
			.transpose()?;
		let target_policies = {
			let binds = inputs.stores.read_binds();
			binds.sub_backend_policies(
				BackendTargetRef::Backend {
					name: name.name.as_ref(),
					namespace: name.namespace.as_ref(),
					section: Some(selected.name.as_ref()),
				},
				resolved.as_ref().map(|r| r.inline_policies.as_slice()),
			)
		};
		let target_name = selected.name.clone();
		let group = McpBackendGroup {
			targets: vec![Arc::new(McpTarget {
				name: target_name.clone(),
				spec: selected.spec.clone(),
				backend: resolved.map(|r| r.backend),
				backend_policies: backend_policies.clone().merge(target_policies),
			})],
			stateful: backend.stateful,
			prefix_mode: backend.prefix_mode,
			failure_mode: backend.failure_mode,
			session_idle_ttl: backend.session_idle_ttl,
		};
		let authorization = backend_policies
			.mcp_authorization
			.clone()
			.unwrap_or_else(|| McpAuthorizationSet::new(RuleSets::from(Vec::new())));
		let mut relay = Relay::new(
			group,
			authorization,
			crate::proxy::httpproxy::PolicyClient::new(inputs.clone()),
		)?;
		relay.mcp_guardrails = backend_policies.mcp_guardrails.clone();
		Ok(Self {
			relay,
			target: target_name,
			ctx: IncomingRequestContext::new(parts),
			initialized: tokio::sync::OnceCell::new(),
			next_id: AtomicI64::new(1),
		})
	}

	fn next_id(&self) -> RequestId {
		RequestId::Number(self.next_id.fetch_add(1, Ordering::Relaxed))
	}

	fn upstream(&self) -> anyhow::Result<&Upstream> {
		self.relay.upstreams.get(&self.target)
	}

	async fn ensure_initialized(&self) -> anyhow::Result<()> {
		self
			.initialized
			.get_or_try_init(|| async {
				let upstream = self.upstream()?;
				let init = rmcp::model::InitializeRequest::new(crate::mcp::session::get_client_info());
				let stream = upstream
					.generic_stream(
						&self.target,
						JsonRpcRequest::new(RequestId::Number(0), init.into()),
						&self.ctx,
					)
					.await
					.map_err(relay_error)?;
				Relay::first_response(stream)
					.await
					.map_err(relay_error)?
					.ok_or_else(|| anyhow::anyhow!("MCP initialize returned no result"))?;
				let initialized = rmcp::model::InitializedNotification {
					method: Default::default(),
					extensions: Default::default(),
				};
				upstream
					.generic_notification(&self.target, initialized.into(), &self.ctx)
					.await
					.map_err(relay_error)?;
				Ok::<(), anyhow::Error>(())
			})
			.await
			.map(|_| ())
	}

	async fn request(&self, request: ClientRequest) -> anyhow::Result<Option<ServerResult>> {
		self.ensure_initialized().await?;
		let upstream = self.upstream()?;
		let stream = upstream
			.generic_stream(
				&self.target,
				JsonRpcRequest::new(self.next_id(), request),
				&self.ctx,
			)
			.await
			.map_err(relay_error)?;
		Relay::first_response(stream).await.map_err(relay_error)
	}

	/// Look the tool up with `tools/list`, following pagination.
	pub(crate) async fn tool_definition(&self, tool: &str) -> anyhow::Result<Option<ToolDefinition>> {
		let mut cursor: Option<String> = None;
		loop {
			let params = cursor
				.take()
				.map(|c| rmcp::model::PaginatedRequestParams::default().with_cursor(Some(c)));
			let request = ClientRequest::ListToolsRequest(rmcp::model::ListToolsRequest {
				params,
				..Default::default()
			});
			let Some(ServerResult::ListToolsResult(result)) = self.request(request).await? else {
				anyhow::bail!("MCP backend did not answer tools/list");
			};
			if let Some(found) = result.tools.iter().find(|t| t.name == tool) {
				return Ok(Some(ToolDefinition {
					description: found.description.as_deref().map(str::to_string),
					input_schema: Value::Object((*found.input_schema).clone()),
				}));
			}
			match result.next_cursor {
				Some(next) if !next.is_empty() => cursor = Some(next),
				_ => return Ok(None),
			}
		}
	}

	/// Call `tool` with `input`, enforcing the backend's authorization rules and guardrails.
	pub(crate) async fn call(
		&self,
		tool: &str,
		input: &Value,
		mcp_log: Option<&AsyncLog<MCPInfo>>,
	) -> anyhow::Result<CallOutcome> {
		self.ensure_initialized().await?;
		let method: Strng = strng::literal!("tools/call");
		let cel = rbac::CelExecWrapper::new(self.ctx.as_request().map(|_| ()));
		let resource = rbac::ResourceType::Tool(rbac::ResourceId::new(
			self.target.to_string(),
			tool.to_string(),
		));
		if !self.relay.policies.validate(&resource, &method, &cel) {
			anyhow::bail!("tool {tool} on {} is not authorized", self.target);
		}
		let arguments = input.as_object().cloned();
		if let Some(log) = mcp_log {
			log.non_atomic_mutate(|l| {
				l.set_tool(self.target.to_string(), tool.to_string());
				l.capture_call_arguments(arguments.clone());
			});
		}
		let mut params = rmcp::model::CallToolRequestParams::new(tool.to_string());
		params.arguments = arguments;
		let mut ctx = self.ctx.clone();
		self
			.relay
			.maybe_run_guardrails_call_request(&self.target, &method, &mut params, &mut ctx)
			.await
			.map_err(relay_error)?;
		let request = JsonRpcRequest::new(
			self.next_id(),
			ClientRequest::CallToolRequest(rmcp::model::CallToolRequest::new(params)),
		);
		let guardrails = self
			.relay
			.build_guardrails_ctx(&request, &ctx, vec![self.target.to_string()]);
		let upstream = self.upstream()?;
		let stream = upstream
			.generic_stream(&self.target, request, &ctx)
			.await
			.map_err(relay_error)?;
		let result = match guardrails {
			Some(g) => Relay::first_response(crate::mcp::handler::wrap_with_guardrails(stream, g)).await,
			None => Relay::first_response(stream).await,
		}
		.map_err(relay_error)?;
		let Some(ServerResult::CallToolResult(result)) = result else {
			anyhow::bail!("MCP backend did not answer tools/call for {tool}");
		};
		if let Some(log) = mcp_log {
			log.non_atomic_mutate(|l| l.capture_call_result(&result));
		}
		let content = result
			.content
			.iter()
			.filter_map(|c| serde_json::to_value(c).ok())
			.collect();
		Ok(CallOutcome {
			content,
			is_error: result.is_error.unwrap_or(false),
		})
	}

	/// End the upstream session, if one was started.
	pub(crate) async fn close(&self) {
		if self.initialized.get().is_none() {
			return;
		}
		if let Ok(upstream) = self.upstream()
			&& let Err(e) = upstream.delete(&self.target, &self.ctx).await
		{
			debug!(target = %self.target, "failed to close MCP session: {e}");
		}
	}
}
