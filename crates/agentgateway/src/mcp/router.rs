use std::sync::Arc;

use agent_core::prelude::Strng;
use axum::response::Response;

use crate::ProxyInputs;
use crate::cel::ContextBuilder;
use crate::http::authorization::RuleSets;
use crate::http::sessionpersistence::Encoder;
use crate::http::*;
use crate::mcp::auth;
use crate::mcp::handler::RelayInputs;
use crate::mcp::session::SessionManager;
use crate::mcp::sse::LegacySSEService;
use crate::mcp::streamablehttp::{StreamableHttpServerConfig, StreamableHttpService};
use crate::mcp::{MCPInfo, McpAuthorizationSet};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::{MustSnapshot, PolicyClient};
use crate::store::{BackendPolicies, Stores};
use crate::telemetry::log::RequestLog;
use crate::types::agent::{
	BackendTargetRef, McpBackend, McpTargetSpec, ResourceName, SimpleBackend, SimpleBackendReference,
};

#[derive(Debug, Clone)]
pub struct App {
	state: Stores,
	session: Arc<SessionManager>,
}

impl App {
	pub fn new(state: Stores, encoder: Encoder) -> Self {
		let session: Arc<SessionManager> = Arc::new(crate::mcp::session::SessionManager::new(encoder));
		Self { state, session }
	}

	pub fn should_passthrough(
		&self,
		backend_policies: &BackendPolicies,
		backend: &McpBackend,
		req: &Request,
	) -> Option<SimpleBackendReference> {
		if backend.targets.len() != 1 {
			return None;
		}

		if backend_policies.mcp_authentication.is_some() {
			return None;
		}
		if !req.uri().path().contains("/.well-known/") {
			return None;
		}
		match backend.targets.first().map(|t| &t.spec) {
			Some(McpTargetSpec::Mcp(s)) => Some(s.backend.clone()),
			Some(McpTargetSpec::Sse(s)) => Some(s.backend.clone()),
			_ => None,
		}
	}

	#[allow(clippy::too_many_arguments)]
	pub async fn serve(
		&self,
		pi: Arc<ProxyInputs>,
		backend_group_name: ResourceName,
		backend: McpBackend,
		backend_policies: BackendPolicies,
		mut req: MustSnapshot<'_>,
		mut log: &mut RequestLog,
	) -> Result<Response, ProxyError> {
		let backends = {
			let binds = self.state.read_binds();
			let nt = backend
				.targets
				.iter()
				.map(|t| {
					let be = t
						.spec
						.backend()
						.map(|b| crate::proxy::resolve_simple_backend_with_policies(b, &pi))
						.transpose()?;
					let inline_pols = be.as_ref().map(|pol| pol.inline_policies.as_slice());
					let sub_backend_target = BackendTargetRef::Backend {
						name: backend_group_name.name.as_ref(),
						namespace: backend_group_name.namespace.as_ref(),
						section: Some(t.name.as_ref()),
					};
					let backend_policies = backend_policies
						.clone()
						.merge(binds.sub_backend_policies(sub_backend_target, inline_pols));
					Ok::<_, ProxyError>(Arc::new(McpTarget {
						name: t.name.clone(),
						spec: t.spec.clone(),
						backend: be.map(|b| b.backend),
						backend_policies,
						always_use_prefix: backend.always_use_prefix,
					}))
				})
				.collect::<Result<Vec<_>, _>>()?;

			McpBackendGroup {
				targets: nt,
				stateful: backend.stateful,
			}
		};
		let sm = self.session.clone();
		let client = PolicyClient { inputs: pi.clone() };
		let authorization_policies = backend_policies
			.mcp_authorization
			.unwrap_or_else(|| McpAuthorizationSet::new(RuleSets::from(Vec::new())));
		let authn = backend_policies.mcp_authentication;

		// Store an empty value, we will populate each field async
		let logy = log.mcp_status.clone();
		logy.store(Some(MCPInfo::default()));
		req.extensions_mut().insert(logy);

		let mut ctx = ContextBuilder::new();
		authorization_policies.register(&mut ctx);
		ctx.maybe_buffer_request_body(&mut req).await;

		// `response` is not valid here, since we run authz first
		// MCP context is added later. The context is inserted after
		// authentication so it can include verified claims

		if let Some(auth) = authn.as_ref()
			&& let Some(resp) = auth::enforce_authentication(&mut req, auth, &client).await?
		{
			return Ok(resp);
		}

		let mut req = req.take_and_snapshot(Some(&mut log))?;
		// This is an unfortunate clone. The request snapshot is intended to be done at the end of the request,
		// so it strips all of the extensions. However, in MCP land its much trickier for us to do this so
		// we snapshot early... but then we lose the extensions. So we do a clone here.
		let snapshot = log.request_snapshot.clone();
		req.extensions_mut().insert(Arc::new(snapshot));
		if req.uri().path() == "/sse" {
			// Legacy handling
			// Assume this is streamable HTTP otherwise
			let sse = LegacySSEService::new(sm);
			Box::pin(sse.handle(
				req,
				RelayInputs {
					backend: backends.clone(),
					policies: authorization_policies.clone(),
					client: client.clone(),
				},
			))
			.await
		} else {
			let streamable = StreamableHttpService::new(
				sm,
				StreamableHttpServerConfig {
					stateful_mode: backend.stateful,
				},
			);
			Box::pin(streamable.handle(
				req,
				RelayInputs {
					backend: backends.clone(),
					policies: authorization_policies.clone(),
					client: client.clone(),
				},
			))
			.await
		}
	}
}

#[derive(Debug, Clone)]
pub struct McpBackendGroup {
	pub targets: Vec<Arc<McpTarget>>,
	pub stateful: bool,
}

#[derive(Debug)]
pub struct McpTarget {
	pub name: Strng,
	pub spec: crate::types::agent::McpTargetSpec,
	pub backend_policies: BackendPolicies,
	pub backend: Option<SimpleBackend>,
	pub always_use_prefix: bool,
}
