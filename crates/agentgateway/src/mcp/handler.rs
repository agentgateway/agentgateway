use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agent_core::prelude::{AssertSize, Strng};
use agent_core::version::BuildInfo;
use futures_core::Stream;
use futures_util::StreamExt;
use http::StatusCode;
use http::request::Parts;
use itertools::Itertools;
use rmcp::ErrorData;
use rmcp::model::{
	CacheScope, ClientNotification, ClientRequest, ConstString, DiscoverResult,
	ExtensionCapabilities, Implementation, JsonRpcNotification, JsonRpcRequest, ListPromptsResult,
	ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
	ProtocolVersion, RequestId, ResultType, ServerCapabilities, ServerInfo, ServerJsonRpcMessage,
	ServerNotification, ServerRequest, ServerResult, SubscriptionFilter,
};
use tracing::{debug, info, warn};

use crate::http::Response;
use crate::http::sessionpersistence::MCPSession;
use crate::mcp;
use crate::mcp::mergestream::{MergeFn, Messages};
use crate::mcp::rbac::{CelExecWrapper, McpAuthorizationSet};
use crate::mcp::router::McpBackendGroup;
use crate::mcp::streamablehttp::{RequestProtocol, ServerSseMessage};
use crate::mcp::subscriptions::ResourceSubscription;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, FailureMode, MCPInfo, apps, mergestream, rbac, upstream};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::{AsyncLog, SpanWriteOnDrop, SpanWriter};
use crate::types::agent::McpPrefixMode;

const DELIMITER: &str = "_";

fn resource_name(prefix_names: bool, target: &str, name: &str) -> String {
	if prefix_names {
		format!("{target}{DELIMITER}{name}")
	} else {
		name.to_string()
	}
}

fn duplicate_names<'a>(enabled: bool, names: impl Iterator<Item = &'a str>) -> HashSet<String> {
	if !enabled {
		return HashSet::new();
	}
	let duplicates = names
		.duplicates()
		.map(str::to_owned)
		.collect::<HashSet<_>>();
	if !duplicates.is_empty() {
		debug!(
			"dropping ambiguous MCP names served by multiple targets: {}",
			duplicates.iter().sorted().join(", ")
		);
	}
	duplicates
}

/// Split per-target list results and, when rejecting duplicates, drop names
/// served by more than one target.
fn per_target_deduped<T>(
	streams: Vec<(Strng, ServerResult)>,
	reject_duplicates: bool,
	extract: impl Fn(ServerResult) -> Vec<T>,
	name: impl for<'a> Fn(&'a T) -> &'a str,
) -> Vec<(Strng, Vec<T>)> {
	let per_target = streams
		.into_iter()
		.map(|(server_name, s)| (server_name, extract(s)))
		.collect_vec();
	let duplicates = duplicate_names(
		reject_duplicates,
		per_target
			.iter()
			.flat_map(|(_, items)| items.iter().map(&name)),
	);
	per_target
		.into_iter()
		.map(|(server_name, items)| {
			let items = items
				.into_iter()
				.filter(|item| !duplicates.contains(name(item)))
				.collect_vec();
			(server_name, items)
		})
		.collect_vec()
}

/// Returns the byte length of an RFC 3986 scheme prefix ending at `:`, if present.
///
/// MCP resource identifiers are URIs, which include hierarchical forms (`http://…`)
/// and opaque forms (`foo:bar`, `urn:uuid:…`). Multiplexing must accept both.
fn uri_scheme_prefix_len(uri: &str) -> Option<usize> {
	let bytes = uri.as_bytes();
	if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
		return None;
	}
	let mut i = 1;
	while i < bytes.len() {
		match bytes[i] {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'-' | b'.' => i += 1,
			b':' => return Some(i),
			_ => return None,
		}
	}
	None
}

fn resource_uri(default_target_name: Option<&String>, target: &str, uri: &str) -> String {
	if default_target_name.is_none() {
		// Apps UI resources must keep their ui:// scheme so hosts still
		// recognize them; the target is carried in the authority instead.
		if let Some(rewritten) = apps::encode_ui_uri(target, uri) {
			return rewritten;
		}
		// Prefix with `service+` for multiplexing whenever the URI has a scheme.
		// e.g. "http://example.com" → "service+http://example.com"
		//      "memo:insights"      → "service+memo:insights"
		if uri_scheme_prefix_len(uri).is_some() {
			format!("{target}+{uri}")
		} else {
			// Scheme-less / relative references stay unchanged; validation handles rejects.
			uri.to_string()
		}
	} else {
		uri.to_string()
	}
}

pub(super) fn rewrite_resource_messages(
	default_target_name: Option<&String>,
	target: &str,
	mut message: ServerJsonRpcMessage,
) -> ServerJsonRpcMessage {
	if let ServerJsonRpcMessage::Notification(notification) = &mut message
		&& let ServerNotification::ResourceUpdatedNotification(resource_updated) =
			&mut notification.notification
	{
		resource_updated.params.uri = resource_uri(
			default_target_name,
			target,
			resource_updated.params.uri.as_str(),
		);
	}
	if let ServerJsonRpcMessage::Response(resp) = &mut message
		&& let ServerResult::ReadResourceResult(read) = &mut resp.result
	{
		for content in &mut read.contents {
			match content {
				rmcp::model::ResourceContents::TextResourceContents { uri, .. }
				| rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => {
					*uri = resource_uri(default_target_name, target, uri);
				},
				_ => {},
			}
		}
	}
	message
}

/// What kind of name is being resolved to a target (`prefixMode: never`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolveKind {
	Tool,
	Prompt,
}

impl ResolveKind {
	fn as_str(&self) -> &'static str {
		match self {
			ResolveKind::Tool => "tool",
			ResolveKind::Prompt => "prompt",
		}
	}

	fn list_request(&self, cursor: Option<String>) -> ClientRequest {
		let params = cursor.map(|c| PaginatedRequestParams::default().with_cursor(Some(c)));
		match self {
			ResolveKind::Tool => ClientRequest::ListToolsRequest(rmcp::model::ListToolsRequest {
				params,
				..Default::default()
			}),
			ResolveKind::Prompt => ClientRequest::ListPromptsRequest(rmcp::model::ListPromptsRequest {
				params,
				..Default::default()
			}),
		}
	}

	fn next_cursor(&self, result: &ServerResult) -> Option<String> {
		match (self, result) {
			(ResolveKind::Tool, ServerResult::ListToolsResult(r)) => r.next_cursor.clone(),
			(ResolveKind::Prompt, ServerResult::ListPromptsResult(r)) => r.next_cursor.clone(),
			_ => None,
		}
	}

	fn contains_name(&self, result: &ServerResult, name: &str) -> bool {
		match (self, result) {
			(ResolveKind::Tool, ServerResult::ListToolsResult(r)) => {
				r.tools.iter().any(|t| t.name == name)
			},
			(ResolveKind::Prompt, ServerResult::ListPromptsResult(r)) => {
				r.prompts.iter().any(|p| p.name == name)
			},
			_ => false,
		}
	}
}

#[derive(Debug, Clone)]
pub struct Relay {
	pub(crate) upstreams: Arc<upstream::UpstreamGroup>,
	pub policies: McpAuthorizationSet,
	pub(crate) mcp_guardrails: Option<Arc<crate::mcp::guardrails::McpGuardrails>>,
	pub(crate) policy_client: PolicyClient,
}

pub struct RelayInputs {
	pub backend: McpBackendGroup,
	pub policies: McpAuthorizationSet,
	pub mcp_guardrails: Option<Arc<crate::mcp::guardrails::McpGuardrails>>,
	pub client: PolicyClient,
}

impl RelayInputs {
	pub fn build_new_connections(self) -> Result<Relay, mcp::Error> {
		let r = Relay::new(self.backend, self.policies, self.client)?;
		Ok(Relay {
			mcp_guardrails: self.mcp_guardrails,
			..r
		})
	}
}

impl Relay {
	pub fn new(
		backend: McpBackendGroup,
		policies: McpAuthorizationSet,
		client: PolicyClient,
	) -> Result<Self, mcp::Error> {
		Ok(Self {
			upstreams: Arc::new(upstream::UpstreamGroup::new(client.clone(), backend)?),
			policies,
			mcp_guardrails: None,
			policy_client: client,
		})
	}
	pub fn with_policies(&self, policies: McpAuthorizationSet) -> Self {
		Self {
			upstreams: self.upstreams.clone(),
			policies,
			mcp_guardrails: self.mcp_guardrails.clone(),
			policy_client: self.policy_client.clone(),
		}
	}

	fn rewrite_outbound_server_messages(
		&self,
		target: &str,
		stream: Messages,
		cel: CelExecWrapper,
	) -> Messages {
		let target = target.to_string();
		let default_target_name = self.upstreams.default_target_name.clone();
		let policies = self.policies.clone();
		stream.map_server_messages(move |message| {
			let message = rewrite_resource_messages(default_target_name.as_ref(), &target, message);

			let mut resource_allowed = |uri: &str| {
				// rewrite_tool_list_ui_meta extracts app URIs from tool metadata, apply RBAC against
				// these UI resources
				policies.validate(
					&rbac::ResourceType::Resource(rbac::ResourceId::new(target.clone(), uri.to_string())),
					&cel,
				)
			};
			apps::rewrite_tool_list_ui_meta(
				default_target_name.is_none(),
				&target,
				&mut resource_allowed,
				message,
			)
		})
	}

	/// Whether names carry no routing information (`prefixMode: never`), so the
	/// owning target can only be found by listing upstreams.
	pub fn needs_resolution(&self) -> bool {
		self.upstreams.is_multiplexing && self.upstreams.prefix_mode == McpPrefixMode::Never
	}

	/// Whether tool/prompt names are exposed to clients with a target prefix.
	fn prefix_names(&self) -> bool {
		self.upstreams.default_target_name.is_none() && !self.needs_resolution()
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

	/// Find the target for an unprefixed name from the corresponding list response.
	pub async fn resolve_resource_name<'a, 'b: 'a>(
		&'a self,
		kind: ResolveKind,
		res: &'b str,
		ctx: &IncomingRequestContext,
	) -> Result<(Cow<'a, str>, &'b str), UpstreamError> {
		if self.needs_resolution() {
			let target = self.resolve_unprefixed(kind, res, ctx).await?;
			return Ok((Cow::Owned(target.to_string()), res));
		}
		let (target, name) = self.parse_resource_name(res)?;
		Ok((Cow::Borrowed(target), name))
	}

	/// Find the single target serving the unprefixed `name` by listing every
	/// target at call time.
	/// TODO cache list results so every tool call/prompt get doesn't require making
	/// tons of extra list calls to every upstream.
	async fn resolve_unprefixed(
		&self,
		kind: ResolveKind,
		name: &str,
		ctx: &IncomingRequestContext,
	) -> Result<Strng, UpstreamError> {
		let futs: Vec<_> = self
			.upstreams
			.iter_named()
			.map(|(target, con)| async move {
				let res = Self::serves_name(&con, kind, name, ctx).await;
				(target, res)
			})
			.collect();

		let mut owner = None;
		for (target, res) in futures::future::join_all(futs).await {
			match res {
				Ok(true) => {
					if owner.is_some() {
						return Err(UpstreamError::InvalidRequest(format!(
							"{} {name} is served by multiple targets",
							kind.as_str()
						)));
					}
					owner = Some(target);
				},
				Ok(false) => {},
				Err(e) => {
					if self.upstreams.failure_mode == FailureMode::FailOpen {
						warn!(
							"upstream '{target}' failed while resolving {} '{name}', skipping: {e}",
							kind.as_str()
						);
					} else {
						return Err(e);
					}
				},
			}
		}

		owner.ok_or_else(|| UpstreamError::InvalidRequest(format!("unknown {} {name}", kind.as_str())))
	}

	/// Page through one target's `kind` list until `name` is found or the pages
	/// run out. `Ok(false)` includes targets that don't support the list method.
	async fn serves_name(
		con: &upstream::Upstream,
		kind: ResolveKind,
		name: &str,
		ctx: &IncomingRequestContext,
	) -> Result<bool, UpstreamError> {
		// Gateway-generated ids: reusing the client's id here would make the upstream
		// see it twice (list probe, then the forwarded call) in one session.
		static RESOLVE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
		// Bounds paging against upstreams that return cursors forever.
		const MAX_LIST_PAGES: usize = 64;
		let mut cursor = None;
		for _ in 0..MAX_LIST_PAGES {
			let seq = RESOLVE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
			let req = JsonRpcRequest::new(
				RequestId::String(format!("agw-resolve-{seq}").into()),
				kind.list_request(cursor),
			);
			let Some(result) = Self::first_response(con.generic_stream(req, ctx).await?).await? else {
				return Ok(false);
			};
			if kind.contains_name(&result, name) {
				return Ok(true);
			}
			cursor = kind.next_cursor(&result);
			if cursor.is_none() {
				return Ok(false);
			}
		}
		Err(UpstreamError::InvalidRequest(format!(
			"exceeded {MAX_LIST_PAGES} pages listing {}s",
			kind.as_str()
		)))
	}

	/// Consume a response stream until the first result, error data, or end.
	/// `Ok(None)` means the target rejected the list method as unsupported.
	async fn first_response(stream: Messages) -> Result<Option<ServerResult>, UpstreamError> {
		let mut stream = std::pin::pin!(stream);
		while let Some(msg) = stream.next().await {
			match msg {
				Ok(ServerJsonRpcMessage::Response(resp)) => return Ok(Some(resp.result)),
				Ok(ServerJsonRpcMessage::Error(err)) => {
					if err.error.code == rmcp::model::ErrorCode::METHOD_NOT_FOUND {
						return Ok(None);
					}
					return Err(UpstreamError::InvalidRequest(err.error.message.to_string()));
				},
				Ok(_) => {},
				Err(e) => return Err(e.into()),
			}
		}
		Err(UpstreamError::Recv)
	}

	/// Reverse of `resource_uri`: extracts the service name and original URI from a
	/// multiplexed URI of the form `service+<original-uri>` (hierarchical or opaque),
	/// or `ui://service+rest` for Apps UI resources.
	pub fn parse_resource_uri<'a>(&'a self, uri: &str) -> Result<(&'a str, String), UpstreamError> {
		if let Some(default) = self.upstreams.default_target_name.as_ref() {
			Ok((default.as_str(), uri.to_string()))
		} else if apps::is_ui_uri(uri) {
			let (service_name, original_uri) = apps::decode_ui_uri(uri)
				.ok_or_else(|| UpstreamError::InvalidRequest("invalid resource URI".to_string()))?;
			let validated_name = self
				.upstreams
				.get_name(service_name)
				.ok_or_else(|| UpstreamError::InvalidRequest(format!("unknown service {service_name}")))?;
			Ok((validated_name, original_uri))
		} else {
			// URI format: "service+<original-uri>" where original-uri is any absolute URI
			let plus_pos = uri
				.find('+')
				.ok_or_else(|| UpstreamError::InvalidRequest("invalid resource URI".to_string()))?;
			let service_name = &uri[..plus_pos];
			let original_uri = &uri[plus_pos + 1..];
			// ui:// resources use the ui://service+rest namespace exclusively
			if apps::is_ui_uri(original_uri) {
				return Err(UpstreamError::InvalidRequest(
					"invalid resource URI".to_string(),
				));
			}
			// Validate that the extracted service name corresponds to a known upstream
			let validated_name = self
				.upstreams
				.get_name(service_name)
				.ok_or_else(|| UpstreamError::InvalidRequest(format!("unknown service {service_name}")))?;
			Ok((validated_name, original_uri.to_string()))
		}
	}

	pub fn get_sessions(&self) -> Option<Vec<MCPSession>> {
		let mut sessions = Vec::with_capacity(self.upstreams.size());
		for (_, us) in self.upstreams.iter_named() {
			sessions.push(us.get_session_state()?);
		}
		Some(sessions)
	}

	pub fn set_sessions(&self, sessions: Vec<MCPSession>) -> anyhow::Result<()> {
		if sessions.iter().all(|session| session.target_name.is_none()) {
			if sessions.len() != self.upstreams.size() {
				anyhow::bail!(
					"session count {} did not match initialized upstreams {}",
					sessions.len(),
					self.upstreams.size()
				);
			}
			for ((_, us), session) in self.upstreams.iter_named().zip(sessions) {
				us.set_session_id(session.session.as_deref(), session.backend);
			}
			return Ok(());
		}

		if sessions.iter().any(|session| session.target_name.is_none()) {
			anyhow::bail!("mixed keyed and unkeyed MCP session state is unsupported");
		}

		// Target-keyed resume is intentionally strict: if the initialized target set changed,
		// failing the resume is safer than binding persisted session state to the wrong target.
		let mut by_target = HashMap::with_capacity(sessions.len());
		for session in sessions {
			let target_name = session
				.target_name
				.clone()
				.expect("checked all sessions are target-keyed above");
			if by_target.insert(target_name.clone(), session).is_some() {
				anyhow::bail!("duplicate persisted session for target {target_name}");
			}
		}

		if by_target.len() != self.upstreams.size() {
			anyhow::bail!(
				"persisted target count {} did not match initialized upstreams {}",
				by_target.len(),
				self.upstreams.size()
			);
		}

		for (target_name, us) in self.upstreams.iter_named() {
			let session = by_target
				.remove(target_name.as_str())
				.ok_or_else(|| anyhow::anyhow!("missing persisted session for target {target_name}"))?;
			us.set_session_id(session.session.as_deref(), session.backend);
		}
		Ok(())
	}
	pub fn is_multiplexing(&self) -> bool {
		self.upstreams.is_multiplexing
	}

	fn build_guardrails_ctx(
		&self,
		r: &JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
		backends: Vec<String>,
	) -> Option<GuardrailsCtx> {
		let ext = self.mcp_guardrails.as_ref()?;
		let method = r.request.method().to_string();
		if !ext.runs_response(&method) {
			// we only need an GuardrailsCtx for response-phase guardrails hooks
			return None;
		}
		Some(GuardrailsCtx {
			ext: ext.clone(),
			method,
			backends,
			client: self.policy_client.clone(),
			req_ctx: Arc::new(ctx.clone()),
		})
	}

	pub(crate) async fn run_guardrails_call_request<P: serde::de::DeserializeOwned>(
		&self,
		ext_ctx: &mut crate::mcp::guardrails::CallRequestCtx<'_>,
		ctx: &mut IncomingRequestContext,
	) -> Result<Option<P>, UpstreamError> {
		use crate::mcp::guardrails::Outcome;
		let Some(ext) = self.mcp_guardrails.as_ref() else {
			return Ok(None);
		};
		let method = ext_ctx.method;
		match crate::mcp::guardrails::run_call_request::<P>(ext, ext_ctx, ctx, &self.policy_client)
			.await
		{
			Outcome::Pass => Ok(None),
			Outcome::Mutated(p) => {
				tracing::debug!(method, "mcpGuardrails: request mutated");
				Ok(Some(p))
			},
			Outcome::Reject(rej) => {
				tracing::debug!(
					method,
					code = rej.code.0,
					message = %rej.message,
					"mcpGuardrails: request rejected",
				);
				Err(UpstreamError::McpGuardrails(rej))
			},
		}
	}

	pub(crate) async fn maybe_run_guardrails_call_request<P>(
		&self,
		backend: &str,
		method: &str,
		params: &mut P,
		ctx: &mut IncomingRequestContext,
	) -> Result<(), UpstreamError>
	where
		P: serde::Serialize + serde::de::DeserializeOwned,
	{
		let Some(ext) = self.mcp_guardrails.as_ref() else {
			return Ok(());
		};
		// Skip the (potentially expensive) params serialization when this method
		// has no request-phase hook configured.
		if !ext.runs_request(method) {
			return Ok(());
		}
		let params_b = serde_json::to_vec(&*params)
			.map_err(|e| UpstreamError::InvalidRequest(format!("serialize {method} params: {e}")))?;
		let backends = [backend.to_string()];
		if let Some(p) = self
			.run_guardrails_call_request::<P>(
				&mut crate::mcp::guardrails::CallRequestCtx {
					backends: &backends,
					method,
					params: Some(params_b.into()),
				},
				ctx,
			)
			.await?
		{
			*params = p;
		}
		Ok(())
	}

	pub fn merge_tools(&self) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let prefix_names = self.prefix_names();
		let reject_duplicates = self.needs_resolution();
		Box::new(move |streams, cel| {
			let per_target = per_target_deduped(
				streams,
				reject_duplicates,
				|s| match s {
					ServerResult::ListToolsResult(ltr) => ltr.tools,
					_ => vec![],
				},
				|tool| tool.name.as_ref(),
			);
			let tools = per_target
				.into_iter()
				.flat_map(|(server_name, tools)| {
					tools
						.into_iter()
						// Apply authorization policies, filtering tools that are not allowed.
						.filter(|t| {
							policies.validate(
								&rbac::ResourceType::Tool(rbac::ResourceId::new(
									server_name.to_string(),
									t.name.to_string(),
								)),
								cel,
							)
						})
						// Rename to handle multiplexing
						.map(|mut t| {
							t.name = Cow::Owned(resource_name(prefix_names, server_name.as_str(), &t.name));
							t
						})
						.collect_vec()
				})
				.collect_vec();
			Ok(
				ListToolsResult {
					tools,
					..Default::default()
				}
				.with_ttl_ms(0)
				.with_cache_scope(CacheScope::Private)
				.into(),
			)
		})
	}

	pub fn merge_initialize(&self, pv: ProtocolVersion, multiplexing: bool) -> Box<MergeFn> {
		let resource_subscribe = self.upstreams.stateful();
		let upstreams = self.upstreams.clone();
		Box::new(move |s, _cel| {
			if !multiplexing {
				// Happy case: we can forward everything
				let res = s.into_iter().next().and_then(|(name, r)| match r {
					ServerResult::InitializeResult(ir) => Some((name, ir)),
					_ => None,
				});
				if let Some((name, ir)) = res {
					upstreams.record_extensions(name.as_str(), ir.capabilities.extensions.as_ref());
					return Ok(ir.into());
				}
				// If we got here in FailOpen mode, it means the only target failed.
				// Return a default info response to keep the client session alive.
				return Ok(
					Self::get_info(
						pv,
						resource_subscribe,
						Vec::new(),
						upstreams.merged_extensions(&HashMap::new()),
					)
					.into(),
				);
			}

			// Multiplexing is more complex. We need to find the lowest protocol version
			// that all servers support and merge instructions from all upstreams.
			let mut lowest_version = pv;
			let mut upstream_instructions: Vec<(String, String)> = Vec::new();

			for (server_name, v) in s {
				if let ServerResult::InitializeResult(r) = v {
					upstreams.record_extensions(server_name.as_str(), r.capabilities.extensions.as_ref());
					if r.protocol_version.to_string() < lowest_version.to_string() {
						lowest_version = r.protocol_version;
					}
					if let Some(instructions) = r.instructions
						&& !instructions.is_empty()
					{
						upstream_instructions.push((server_name.to_string(), instructions));
					}
				}
			}

			Ok(
				Self::get_info(
					lowest_version,
					resource_subscribe,
					upstream_instructions,
					upstreams.merged_extensions(&HashMap::new()),
				)
				.into(),
			)
		})
	}

	pub fn merge_discover(&self, multiplexing: bool) -> Box<MergeFn> {
		let resource_subscribe = self.upstreams.stateful();
		let upstreams = self.upstreams.clone();
		Box::new(move |s, _cel| {
			// Single server: forward the upstream's advertised versions untouched. The gateway
			// does no version translation, so an old-only upstream must surface as-is and let
			// the client fall back to `initialize`. Multiplexed: advertise the intersection.
			if !multiplexing {
				let res = s.into_iter().next().and_then(|(_, r)| match r {
					ServerResult::DiscoverResult(dr) => Some(dr),
					_ => None,
				});
				if let Some(dr) = res {
					// Cache hints describe the gateway's presented server, not the upstream.
					// Gateway routing/backend config can change without client invalidation.
					return Ok(dr.with_cache(0, CacheScope::Private).into());
				}
				// In FailOpen, a default discovery response lets the client continue negotiation
				// when no upstream discovery response is available.
				// Include the recorded extensions from initialize responses.
				return Ok(
					Self::get_discovery(
						resource_subscribe,
						Vec::new(),
						upstreams.merged_extensions(&HashMap::new()),
					)
					.into(),
				);
			}

			let mut upstream_instructions: Vec<(String, String)> = Vec::new();
			let mut supported_versions = ProtocolVersion::KNOWN_VERSIONS.to_vec();
			let mut upstream_extensions: HashMap<Strng, ExtensionCapabilities> = HashMap::new();
			for (server_name, v) in s {
				if let ServerResult::DiscoverResult(mut r) = v {
					if let Some(ext) = r.capabilities.extensions.take()
						&& !ext.is_empty()
					{
						upstream_extensions.insert(server_name.clone(), ext);
					}
					supported_versions.retain(|version| r.supported_versions.contains(version));
					if let Some(instructions) = r.instructions
						&& !instructions.is_empty()
					{
						upstream_instructions.push((server_name.to_string(), instructions));
					}
				}
			}

			let mut discover = Self::get_discovery(
				resource_subscribe,
				upstream_instructions,
				upstreams.merged_extensions(&upstream_extensions),
			);
			discover.supported_versions = supported_versions;
			Ok(discover.into())
		})
	}

	pub fn merge_prompts(&self) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let prefix_names = self.prefix_names();
		let reject_duplicates = self.needs_resolution();
		Box::new(move |streams, cel| {
			let per_target = per_target_deduped(
				streams,
				reject_duplicates,
				|s| match s {
					ServerResult::ListPromptsResult(lpr) => lpr.prompts,
					_ => vec![],
				},
				|prompt| prompt.name.as_str(),
			);
			let prompts = per_target
				.into_iter()
				.flat_map(|(server_name, prompts)| {
					prompts
						.into_iter()
						.filter(|p| {
							policies.validate(
								&rbac::ResourceType::Prompt(rbac::ResourceId::new(
									server_name.to_string(),
									p.name.to_string(),
								)),
								cel,
							)
						})
						.map(|mut p| {
							p.name = resource_name(prefix_names, server_name.as_str(), &p.name);
							p
						})
						.collect_vec()
				})
				.collect_vec();
			Ok(
				ListPromptsResult {
					prompts,
					..Default::default()
				}
				.with_ttl_ms(0)
				.with_cache_scope(CacheScope::Private)
				.into(),
			)
		})
	}
	pub fn merge_resources(&self) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.upstreams.default_target_name.clone();
		Box::new(move |streams, cel| {
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
								cel,
							)
						})
						// Prefix URI with service name when multiplexing to avoid conflicts
						.map(|mut r| {
							r.uri = resource_uri(default_target_name.as_ref(), server_name.as_str(), &r.uri);
							r
						})
						.collect_vec()
				})
				.collect_vec();
			Ok(
				ListResourcesResult {
					resources,
					..Default::default()
				}
				.with_ttl_ms(0)
				.with_cache_scope(CacheScope::Private)
				.into(),
			)
		})
	}
	pub fn merge_resource_templates(&self) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.upstreams.default_target_name.clone();
		Box::new(move |streams, cel| {
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
								cel,
							)
						})
						// Prefix uri_template with service name when multiplexing
						.map(|mut rt| {
							rt.uri_template = resource_uri(
								default_target_name.as_ref(),
								server_name.as_str(),
								&rt.uri_template,
							);
							rt
						})
						.collect_vec()
				})
				.collect_vec();
			Ok(
				ListResourceTemplatesResult {
					resource_templates,
					..Default::default()
				}
				.with_ttl_ms(0)
				.with_cache_scope(CacheScope::Private)
				.into(),
			)
		})
	}
	pub fn merge_empty(&self) -> Box<MergeFn> {
		Box::new(move |_, _cel| Ok(rmcp::model::ServerResult::empty(())))
	}

	/// Opens the upstream streams shared by `subscriptions/listen` and request fanout.
	/// The request-phase mcpGuardrails hook runs once over the shared context before
	/// any upstream is contacted.
	async fn fanout_open_streams(
		&self,
		r: &JsonRpcRequest<ClientRequest>,
		ctx: &mut IncomingRequestContext,
		target_names: Option<Vec<String>>,
		request_for_target: impl Fn(&str, &JsonRpcRequest<ClientRequest>) -> JsonRpcRequest<ClientRequest>,
	) -> Result<(Vec<(Strng, Messages)>, Option<Vec<String>>), UpstreamError> {
		let selected_upstreams = self
			.upstreams
			.iter_named()
			.filter(|(name, _)| {
				target_names
					.as_ref()
					.is_none_or(|targets| targets.iter().any(|target| target == name.as_str()))
			})
			.collect::<Vec<_>>();
		if selected_upstreams.is_empty() {
			return Err(UpstreamError::Unavailable(
				"no upstreams available".to_string(),
			));
		}

		// The fanout-wide mcpGuardrails hook receives every selected backend.
		let service_names = self.mcp_guardrails.as_ref().map(|_| {
			selected_upstreams
				.iter()
				.map(|(n, _)| n.to_string())
				.collect::<Vec<_>>()
		});

		// params is None because fanout has no single request body to rewrite.
		if let Some(ext) = self.mcp_guardrails.as_ref() {
			let outcome = Box::pin(
				crate::mcp::guardrails::run_call_request::<serde_json::Value>(
					ext,
					&mut crate::mcp::guardrails::CallRequestCtx {
						backends: service_names.as_deref().unwrap_or_default(),
						method: r.request.method(),
						params: None,
					},
					ctx,
					&self.policy_client,
				)
				.assert_size::<{ 4 * 1024 }>(),
			)
			.await;
			if let crate::mcp::guardrails::Outcome::Reject(rej) = outcome {
				return Err(UpstreamError::McpGuardrails(rej));
			}
		}

		let futs: Vec<_> = selected_upstreams
			.into_iter()
			.map(|(name, con)| {
				let r = request_for_target(name.as_str(), r);
				let ctx = &*ctx;
				async move { (name, con.generic_stream(r, ctx).await) }
			})
			.collect();
		let fut_results = futures::future::join_all(futs).await;

		let mut streams = Vec::new();
		let mut last_error = None;
		for (name, result) in fut_results {
			match result {
				Ok(s) => streams.push((name, s)),
				Err(e) => {
					// FailOpen skips pre-stream failures. FailClosed fails before any frame streams.
					if self.upstreams.failure_mode == FailureMode::FailOpen {
						warn!("upstream '{}' failed during fanout, skipping: {}", name, e);
						last_error = Some(e);
					} else {
						return Err(e);
					}
				},
			}
		}
		if streams.is_empty() {
			// Request fanout has no transport fallback or generic synthetic success.
			return Err(
				last_error
					.unwrap_or_else(|| UpstreamError::Unavailable("no upstreams available".to_string())),
			);
		}
		Ok((streams, service_names))
	}

	/// Handles `subscriptions/listen` with an ACK derived from every selected upstream.
	///
	/// `client_filter` keeps service+ URIs for the downstream ACK. `resource_subs` carries the
	/// owning target and rewritten upstream URI for each of them.
	pub async fn send_subscriptions_listen(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		mut ctx: IncomingRequestContext,
		client_filter: SubscriptionFilter,
		resource_subs: Vec<ResourceSubscription>,
	) -> Result<Response, UpstreamError> {
		use futures_util::StreamExt;

		use super::subscriptions::{
			client_filter_from_accepted, filter_and_tag_listen_notification, prepare_listen_streams,
			synthesize_listen_ack,
		};
		let id = r.id.clone();
		let has_global_filter = client_filter.tools_list_changed == Some(true)
			|| client_filter.prompts_list_changed == Some(true)
			|| client_filter.resources_list_changed == Some(true);
		// Resource-only listens go to just the owning targets; any global filter widens the
		// fanout to every upstream (non-owners then get a filter without resource URIs).
		let targets = (!has_global_filter && !resource_subs.is_empty()).then(|| {
			resource_subs.iter().fold(Vec::new(), |mut targets, sub| {
				if !targets.contains(&sub.owner) {
					targets.push(sub.owner.clone());
				}
				targets
			})
		});
		let (streams, service_names) = self
			.fanout_open_streams(&r, &mut ctx, targets, |target, r| {
				let mut r = r.clone();
				if let ClientRequest::SubscriptionsListenRequest(slr) = &mut r.request {
					slr.params.notifications =
						listen_filter_for_target(&client_filter, &resource_subs, target);
				}
				r
			})
			.await?;

		let prepared_inputs = streams
			.into_iter()
			.map(|(name, stream)| {
				let filter = listen_filter_for_target(&client_filter, &resource_subs, name.as_str());
				(name, stream, filter)
			})
			.collect();
		let (prepared, accepted_filter) =
			match prepare_listen_streams(prepared_inputs, self.upstreams.failure_mode).await {
				Ok(prepared) => prepared,
				Err(error) => {
					let error_id = id.clone();
					return respond_with_guardrails(
						id,
						futures::stream::once(async move { error.into_downstream_message(error_id) }),
						service_names.and_then(|sn| self.build_guardrails_ctx(&r, &ctx, sn)),
						None,
						&ctx,
					);
				},
			};

		let ack_filter =
			client_filter_from_accepted(&client_filter, &resource_subs, &accepted_filter, &prepared);
		let ack = synthesize_listen_ack(id.clone(), ack_filter);
		let pipelines = prepared
			.into_iter()
			.map(|prepared| {
				let name = prepared.target;
				let stream = prepared.stream;
				let filter = prepared.accepted_filter;
				let target = name.to_string();
				let sub_id = id.clone();
				let default_target_name = self.upstreams.default_target_name.clone();
				(
					name,
					stream.filter_map_messages_result(move |msg| {
						filter_and_tag_listen_notification(
							msg,
							default_target_name.as_ref(),
							&target,
							&filter,
							&sub_id,
						)
					}),
				)
			})
			.collect::<Vec<_>>();

		let merged =
			mergestream::MergeStream::new_without_merge(pipelines, self.upstreams.failure_mode);
		let body = futures::stream::once(async move { Ok(ack) }).chain(merged);

		respond_with_guardrails(
			id,
			body,
			service_names.and_then(|sn| self.build_guardrails_ctx(&r, &ctx, sn)),
			None,
			&ctx,
		)
	}
	pub async fn send_single(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		service_name: &str,
		mcp_log: Option<AsyncLog<MCPInfo>>,
	) -> Result<Response, UpstreamError> {
		let id = r.id.clone();
		let Ok(us) = self.upstreams.get(service_name) else {
			return Err(UpstreamError::InvalidRequest(format!(
				"unknown service {service_name}"
			)));
		};
		let guardrails = self.build_guardrails_ctx(&r, &ctx, vec![service_name.to_string()]);
		let cel = CelExecWrapper::new(ctx.as_request().map(|_| ()));
		let stream = self.rewrite_outbound_server_messages(
			service_name,
			Box::pin(us.generic_stream(r, &ctx).assert_size::<{ 3 * 1024 }>()).await?,
			cel,
		);

		respond_with_guardrails(id, stream, guardrails, mcp_log, &ctx)
	}
	pub async fn send_fanout_deletion(
		&self,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		let futs: Vec<_> = self
			.upstreams
			.iter_named()
			.map(|(name, con)| {
				let ctx = &ctx;
				async move { (name, con.delete(ctx).await) }
			})
			.collect();

		let fut_results = futures::future::join_all(futs).await;

		for (name, result) in fut_results {
			match result {
				Ok(_) => {},
				Err(e) => {
					if self.upstreams.failure_mode == FailureMode::FailOpen {
						warn!(
							"upstream '{}' failed during deletion, skipping: {}",
							name, e
						);
					} else {
						return Err(e);
					}
				},
			}
		}
		Ok(accepted_response())
	}
	pub async fn send_fanout_get(
		&self,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		let mut streams = Vec::new();
		let mut unsupported_get_streams = 0;

		let futs: Vec<_> = self
			.upstreams
			.iter_named()
			.map(|(name, con)| {
				let ctx = &ctx;
				async move { (name, con.get_event_stream(ctx).await) }
			})
			.collect();

		let fut_results = futures::future::join_all(futs).await;

		let cel = CelExecWrapper::new(ctx.as_request().map(|_| ()));
		for (name, result) in fut_results {
			match result {
				Ok(s) => {
					let s = self.rewrite_outbound_server_messages(name.as_str(), s, cel.clone());
					streams.push((name, s));
				},
				Err(e) => {
					let is_405 = if let UpstreamError::Http(ClientError::Status(ref r)) = e
						&& r.status() == StatusCode::METHOD_NOT_ALLOWED
					{
						true
					} else {
						false
					};
					if is_405 && self.upstreams.is_multiplexing {
						debug!("upstream '{}' does not support GET stream, skipping", name);
						unsupported_get_streams += 1;
						continue;
					}
					if self.upstreams.failure_mode == FailureMode::FailOpen {
						if !is_405 {
							// per spec, a 405 is a valid response to say a GET stream is not supported so avoid log spam.
							warn!("upstream '{}' failed for GET stream, skipping: {}", name, e);
						} else {
							debug!("upstream '{}' failed for GET stream, skipping: {}", name, e);
						}
					} else {
						return Err(e);
					}
				},
			}
		}

		if streams.is_empty() {
			if unsupported_get_streams > 0 && unsupported_get_streams == self.upstreams.size() {
				return Err(crate::proxy::ProxyError::MCP(mcp::Error::GetStreamNotSupported).into());
			}
			// FailClosed: unreachable — InitializeRequest would have failed with NoBackends.
			// FailOpen: keep the SSE connection open so legacy SSE clients do not immediately
			// reconnect in a tight loop after all upstream GET streams disappear.
			return messages_to_response(
				RequestId::Number(0),
				Messages::pending(),
				None,
				ctx_downstream_modern(&ctx),
			);
		}

		let ms = mergestream::MergeStream::new_without_merge(streams, self.upstreams.failure_mode);
		messages_to_response(RequestId::Number(0), ms, None, ctx_downstream_modern(&ctx))
	}

	pub async fn send_fanout(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
	) -> Result<Response, UpstreamError> {
		self.send_fanout_to(r, ctx, merge, None).await
	}

	pub async fn send_fanout_to(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		mut ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
		target_names: Option<Vec<String>>,
	) -> Result<Response, UpstreamError> {
		let id = r.id.clone();
		let (streams, service_names) = self
			.fanout_open_streams(&r, &mut ctx, target_names, |_, r| r.clone())
			.await?;

		let cel = CelExecWrapper::new(ctx.as_request().map(|_| ()));
		let streams = streams
			.into_iter()
			.map(|(name, s)| {
				let s = self.rewrite_outbound_server_messages(name.as_str(), s, cel.clone());
				(name, s)
			})
			.collect::<Vec<_>>();

		let ms =
			mergestream::MergeStream::new(streams, id.clone(), merge, cel, self.upstreams.failure_mode);

		// Response-phase hook runs once on the merged (muxed) result.
		respond_with_guardrails(
			id,
			ms,
			service_names.and_then(|sn| self.build_guardrails_ctx(&r, &ctx, sn)),
			None,
			&ctx,
		)
	}
	pub async fn send_notification(
		&self,
		r: JsonRpcNotification<ClientNotification>,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		let futs: Vec<_> = self
			.upstreams
			.iter_named()
			.map(|(name, con)| {
				let notification = r.notification.clone();
				let ctx = &ctx;
				async move { (name, con.generic_notification(notification, ctx).await) }
			})
			.collect();

		let fut_results = futures::future::join_all(futs).await;

		for (name, result) in fut_results {
			match result {
				Ok(_) => {},
				Err(e) => {
					if self.upstreams.failure_mode == FailureMode::FailOpen {
						warn!(
							"upstream '{}' failed during notification, skipping: {}",
							name, e
						);
					} else {
						return Err(e);
					}
				},
			}
		}

		Ok(accepted_response())
	}

	pub async fn send_notification_single(
		&self,
		r: ClientNotification,
		ctx: IncomingRequestContext,
		service_name: &str,
	) -> Result<Response, UpstreamError> {
		let Ok(us) = self.upstreams.get(service_name) else {
			return Err(UpstreamError::InvalidRequest(format!(
				"unknown service {service_name}"
			)));
		};
		us.generic_notification(r, &ctx).await?;
		Ok(accepted_response())
	}

	fn get_info(
		pv: ProtocolVersion,
		resource_subscribe: bool,
		upstream_instructions: Vec<(String, String)>,
		extensions: Option<ExtensionCapabilities>,
	) -> ServerInfo {
		let capabilities = {
			// Prompts are supported with multiplexing using proxy-prefixed names.
			// Resources are supported with multiplexing using service+<uri> prefixing.
			let mut builder = ServerCapabilities::builder()
				.enable_tools()
				.enable_tool_list_changed()
				.enable_prompts()
				.enable_prompts_list_changed()
				.enable_resources()
				.enable_resources_list_changed();
			if resource_subscribe {
				builder = builder.enable_resources_subscribe();
			}
			let mut capabilities = builder.build();
			capabilities.extensions = extensions;
			capabilities
		};
		let gateway_preamble = "This server is a gateway to a set of mcp servers. It is responsible for routing requests to the correct server and aggregating the results.";
		let instructions = if upstream_instructions.is_empty() {
			Some(gateway_preamble.to_string())
		} else {
			let mut merged = String::from(gateway_preamble);
			for (server_name, instruction) in &upstream_instructions {
				merged.push_str(&format!("\n\n[{server_name}]\n{instruction}"));
			}
			Some(merged)
		};
		ServerInfo::new(capabilities)
			.with_protocol_version(pv)
			.with_server_info(Implementation::new(
				"agentgateway",
				BuildInfo::new().version.to_string(),
			))
			.with_instructions(instructions.unwrap_or_default())
	}

	fn get_discovery(
		resource_subscribe: bool,
		upstream_instructions: Vec<(String, String)>,
		extensions: Option<ExtensionCapabilities>,
	) -> DiscoverResult {
		let info = Self::get_info(
			ProtocolVersion::default(),
			resource_subscribe,
			upstream_instructions,
			extensions,
		);
		DiscoverResult::new(ProtocolVersion::KNOWN_VERSIONS.to_vec(), info.capabilities)
			.with_server_info(info.server_info)
			.with_instructions(info.instructions.unwrap_or_default())
			// Discovery is immediately stale because the gateway has no way to
			// invalidate clients when backend membership or routing config changes.
			.with_cache(0, CacheScope::Private)
	}
}

pub fn setup_request_log(
	http: Parts,
	span_name: &str,
) -> (SpanWriteOnDrop, AsyncLog<MCPInfo>, CelExecWrapper) {
	let log = http
		.extensions
		.get::<AsyncLog<MCPInfo>>()
		.cloned()
		.unwrap_or_default();

	let tracer = http
		.extensions
		.get::<SpanWriter>()
		.cloned()
		.unwrap_or_default();
	let cel = CelExecWrapper::new(::http::Request::from_parts(http, ()));
	let _span = tracer.start(span_name.to_string());
	(_span, log, cel)
}

pub(crate) struct GuardrailsCtx {
	pub ext: Arc<crate::mcp::guardrails::McpGuardrails>,
	pub method: String,
	pub backends: Vec<String>,
	pub client: PolicyClient,
	pub req_ctx: Arc<IncomingRequestContext>,
}

pub(super) fn messages_to_response(
	id: RequestId,
	stream: impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
	mcp_log: Option<AsyncLog<MCPInfo>>,
	downstream_modern: bool,
) -> Result<Response, UpstreamError> {
	Ok(mcp::session::sse_stream_response(
		into_sse_stream(id, stream, mcp_log, downstream_modern),
		None,
	))
}

fn respond_with_guardrails(
	id: RequestId,
	stream: impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
	guardrails: Option<GuardrailsCtx>,
	mcp_log: Option<AsyncLog<MCPInfo>>,
	ctx: &IncomingRequestContext,
) -> Result<Response, UpstreamError> {
	match guardrails {
		Some(guardrails) => messages_to_response(
			id,
			wrap_with_guardrails(stream, guardrails),
			mcp_log,
			ctx_downstream_modern(ctx),
		),
		None => messages_to_response(id, stream, mcp_log, ctx_downstream_modern(ctx)),
	}
}

pub(crate) fn ctx_downstream_modern(ctx: &IncomingRequestContext) -> bool {
	ctx
		.extensions()
		.get::<RequestProtocol>()
		.is_some_and(RequestProtocol::is_modern)
}

fn listen_filter_for_target(
	client_filter: &SubscriptionFilter,
	resource_subs: &[ResourceSubscription],
	target: &str,
) -> SubscriptionFilter {
	let mut filter = client_filter.clone();
	let mine = resource_subs
		.iter()
		.filter(|sub| sub.owner == target)
		.map(|sub| sub.upstream_uri.clone())
		.collect::<Vec<_>>();
	filter.resource_subscriptions = (!mine.is_empty()).then_some(mine);
	filter
}

fn wrap_with_guardrails(
	stream: impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
	guardrails: GuardrailsCtx,
) -> impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static {
	use futures_util::StreamExt;
	let guardrails = Arc::new(guardrails);
	stream.then(move |rpc| {
		let ctx = guardrails.clone();
		async move {
			match rpc {
				Ok(mut rpc) => {
					if let Some(scrubbed) = apply_guardrails_response_intercept(&ctx, &rpc).await {
						rpc = scrubbed;
					}
					Ok(rpc)
				},
				Err(e) => Err(e),
			}
		}
	})
}

fn into_sse_stream(
	request_id: RequestId,
	stream: impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
	mcp_log: Option<AsyncLog<MCPInfo>>,
	downstream_modern: bool,
) -> impl Stream<Item = ServerSseMessage> + Send + 'static {
	use futures_util::StreamExt;
	stream
		.scan((false, false), move |st, rpc| {
			let (errored, terminal_seen) = st;
			std::future::ready((!*errored).then(|| {
				let msg: Option<ServerJsonRpcMessage> = match rpc {
					Ok(ServerJsonRpcMessage::Request(req)) if downstream_modern => {
						reject_direct_request(req, &request_id, mcp_log.as_ref(), *terminal_seen, errored)
					},
					Ok(rpc) => {
						let rpc =
							normalize_outbound_for_protocol(with_gateway_cache_policy(rpc), downstream_modern);
						if !*terminal_seen {
							*terminal_seen = capture_terminal_mcp_payload(mcp_log.as_ref(), &request_id, &rpc);
						}
						Some(rpc)
					},
					Err(e) => {
						*errored = true;
						if *terminal_seen {
							None
						} else {
							let msg = ServerJsonRpcMessage::error(
								ErrorData::internal_error(e.to_string(), None),
								Some(request_id.clone()),
							);
							*terminal_seen = capture_terminal_mcp_payload(mcp_log.as_ref(), &request_id, &msg);
							Some(msg)
						}
					},
				};
				// TODO: is it ok to have no event_id here?
				msg.map(|message| ServerSseMessage {
					event_id: None,
					message: Arc::new(message),
				})
			}))
		})
		.flat_map(futures_util::stream::iter)
}

// Modern servers must return input_required instead of direct requests.
fn reject_direct_request(
	req: JsonRpcRequest<ServerRequest>,
	request_id: &RequestId,
	mcp_log: Option<&AsyncLog<MCPInfo>>,
	terminal_seen: bool,
	errored: &mut bool,
) -> Option<ServerJsonRpcMessage> {
	info!(
		"upstream sent a direct server-to-client request on a modern stream: {}",
		server_request_method(&req.request)
	);
	*errored = true;
	if terminal_seen {
		return None;
	}
	let msg = ServerJsonRpcMessage::error(
		ErrorData::internal_error(
			"gateway cannot route direct server-to-client requests; return an input_required result instead",
			None,
		),
		Some(request_id.clone()),
	);
	capture_terminal_mcp_payload(mcp_log, request_id, &msg);
	Some(msg)
}

fn server_request_method(request: &ServerRequest) -> &str {
	match request {
		ServerRequest::PingRequest(r) => r.method.as_str(),
		ServerRequest::CreateMessageRequest(r) => r.method.as_str(),
		ServerRequest::ListRootsRequest(r) => r.method.as_str(),
		ServerRequest::ElicitRequest(r) => r.method.as_str(),
		ServerRequest::CustomRequest(r) => r.method.as_str(),
	}
}

fn normalize_outbound_for_protocol(
	mut msg: ServerJsonRpcMessage,
	downstream_modern: bool,
) -> ServerJsonRpcMessage {
	let ServerJsonRpcMessage::Response(resp) = &mut msg else {
		return msg;
	};

	match &mut resp.result {
		ServerResult::DiscoverResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern);
			normalize_cache_fields(&mut r.ttl_ms, &mut r.cache_scope, downstream_modern);
		},
		ServerResult::ListToolsResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern);
			normalize_cache_fields(&mut r.ttl_ms, &mut r.cache_scope, downstream_modern);
		},
		ServerResult::ListPromptsResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern);
			normalize_cache_fields(&mut r.ttl_ms, &mut r.cache_scope, downstream_modern);
		},
		ServerResult::ListResourcesResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern);
			normalize_cache_fields(&mut r.ttl_ms, &mut r.cache_scope, downstream_modern);
		},
		ServerResult::ListResourceTemplatesResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern);
			normalize_cache_fields(&mut r.ttl_ms, &mut r.cache_scope, downstream_modern);
		},
		ServerResult::ReadResourceResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern);
			normalize_cache_fields(&mut r.ttl_ms, &mut r.cache_scope, downstream_modern);
		},
		ServerResult::InitializeResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern)
		},
		ServerResult::CompleteResult(r) => normalize_result_type(&mut r.result_type, downstream_modern),
		ServerResult::GetPromptResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern)
		},
		ServerResult::ElicitResult(r) => normalize_result_type(&mut r.result_type, downstream_modern),
		ServerResult::CreateTaskResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern)
		},
		ServerResult::ListTasksResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern)
		},
		ServerResult::GetTaskResult(r) => normalize_result_type(&mut r.result_type, downstream_modern),
		ServerResult::CancelTaskResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern)
		},
		ServerResult::SubscriptionsListenResult(r) => {
			normalize_result_type(&mut r.result_type, downstream_modern)
		},
		ServerResult::CallToolResult(r) => normalize_result_type(&mut r.result_type, downstream_modern),
		ServerResult::EmptyResult(r) => normalize_result_type(&mut r.result_type, downstream_modern),
		ServerResult::GetTaskPayloadResult(r) => {
			if !downstream_modern {
				strip_protocol_result_fields(&mut r.0)
			}
		},
		ServerResult::CustomResult(r) => {
			if !downstream_modern {
				strip_protocol_result_fields(&mut r.0)
			}
		},
		ServerResult::InputRequiredResult(_) => {},
	}

	msg
}

fn normalize_result_type(result_type: &mut Option<ResultType>, downstream_modern: bool) {
	if downstream_modern {
		result_type.get_or_insert(ResultType::COMPLETE);
	} else {
		*result_type = None;
	}
}

fn normalize_cache_fields(
	ttl_ms: &mut Option<u64>,
	cache_scope: &mut Option<CacheScope>,
	downstream_modern: bool,
) {
	if !downstream_modern {
		*ttl_ms = None;
		*cache_scope = None;
	}
}

fn strip_protocol_result_fields(value: &mut serde_json::Value) {
	let Some(obj) = value.as_object_mut() else {
		return;
	};
	obj.remove("resultType");
	obj.remove("ttlMs");
	obj.remove("cacheScope");
}

fn with_gateway_cache_policy(mut msg: ServerJsonRpcMessage) -> ServerJsonRpcMessage {
	let ServerJsonRpcMessage::Response(resp) = &mut msg else {
		return msg;
	};

	// Cache hints must describe the gateway-visible result, not the upstream's result.
	// For now, keep all cacheable MCP responses immediately stale/private because
	// routing config, backend membership, and authz filtering can change without a
	// client invalidation path. A future opt-in can allow positive TTLs when no
	// per-user/authz-dependent filtering applies.
	match &mut resp.result {
		ServerResult::DiscoverResult(r) => {
			r.ttl_ms = Some(0);
			r.cache_scope = Some(CacheScope::Private);
		},
		ServerResult::ListToolsResult(r) => {
			r.ttl_ms = Some(0);
			r.cache_scope = Some(CacheScope::Private);
		},
		ServerResult::ListPromptsResult(r) => {
			r.ttl_ms = Some(0);
			r.cache_scope = Some(CacheScope::Private);
		},
		ServerResult::ListResourcesResult(r) => {
			r.ttl_ms = Some(0);
			r.cache_scope = Some(CacheScope::Private);
		},
		ServerResult::ListResourceTemplatesResult(r) => {
			r.ttl_ms = Some(0);
			r.cache_scope = Some(CacheScope::Private);
		},
		ServerResult::ReadResourceResult(r) => {
			r.ttl_ms = Some(0);
			r.cache_scope = Some(CacheScope::Private);
		},
		_ => {},
	}

	msg
}

async fn apply_guardrails_response_intercept(
	ctx: &GuardrailsCtx,
	msg: &ServerJsonRpcMessage,
) -> Option<ServerJsonRpcMessage> {
	use crate::mcp::guardrails::Outcome;
	// The stream is request-scoped, so the only Response on it is the terminal.
	let ServerJsonRpcMessage::Response(resp) = msg else {
		return None;
	};
	let json: bytes::Bytes = match serde_json::to_vec(&resp.result) {
		Ok(v) => v.into(),
		Err(e) => {
			// Fail the response rather than skip a hook the operator configured,
			// matching the request side's handling of serialize failures.
			tracing::warn!(error = %e, "mcpGuardrails: failed to serialize result for inspection");
			return Some(ServerJsonRpcMessage::error(
				ErrorData::internal_error(format!("mcpGuardrails: serialize result: {e}"), None),
				Some(resp.id.clone()),
			));
		},
	};
	match crate::mcp::guardrails::run_response(
		&ctx.ext,
		&ctx.method,
		&ctx.backends,
		json,
		&ctx.req_ctx,
		&ctx.client,
	)
	.await
	{
		Outcome::Pass => None,
		Outcome::Mutated(new_result) => {
			Some(ServerJsonRpcMessage::response(new_result, resp.id.clone()))
		},
		Outcome::Reject(rej) => Some(ServerJsonRpcMessage::error(rej, Some(resp.id.clone()))),
	}
}

fn capture_terminal_mcp_payload(
	log: Option<&AsyncLog<MCPInfo>>,
	request_id: &RequestId,
	message: &ServerJsonRpcMessage,
) -> bool {
	match message {
		ServerJsonRpcMessage::Response(response) if response.id == *request_id => {
			if let ServerResult::CallToolResult(result) = &response.result
				&& let Some(log) = log
			{
				log.non_atomic_mutate(|mcp| mcp.capture_call_result(result));
			}
			true
		},
		ServerJsonRpcMessage::Error(error) if error.id.as_ref() == Some(request_id) => {
			if let Some(log) = log {
				log.non_atomic_mutate(|mcp| mcp.capture_call_error(&error.error));
			}
			true
		},
		_ => false,
	}
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}

#[cfg(test)]
mod tests {
	use futures_util::{StreamExt, stream};
	use rmcp::model::{CallToolResult, ListResourcesResult, ListToolsResult};
	use serde_json::json;

	use super::*;

	#[test]
	fn uri_scheme_prefix_accepts_hierarchical_and_opaque() {
		assert_eq!(uri_scheme_prefix_len("http://example.com/x"), Some(4));
		assert_eq!(uri_scheme_prefix_len("svn+ssh://host/repo"), Some(7));
		assert_eq!(uri_scheme_prefix_len("memo:insights"), Some(4));
		assert_eq!(uri_scheme_prefix_len("urn:uuid:1234"), Some(3));
		assert_eq!(uri_scheme_prefix_len("relative/path"), None);
		assert_eq!(uri_scheme_prefix_len("/absolute/path"), None);
		assert_eq!(uri_scheme_prefix_len(""), None);
	}

	#[test]
	fn resource_uri_multiplexes_opaque_and_hierarchical() {
		let single = Some("only".to_string());
		assert_eq!(
			resource_uri(single.as_ref(), "svc", "memo:insights"),
			"memo:insights"
		);
		assert_eq!(
			resource_uri(None, "svc", "http://example.com/x"),
			"svc+http://example.com/x"
		);
		assert_eq!(
			resource_uri(None, "svc", "memo:insights"),
			"svc+memo:insights"
		);
		assert_eq!(
			resource_uri(None, "svc", "svn+ssh://host/repo"),
			"svc+svn+ssh://host/repo"
		);
		assert_eq!(resource_uri(None, "svc", "relative/path"), "relative/path");
		// Apps: ui:// must stay on the left; never become svc+ui://…
		assert_eq!(
			resource_uri(None, "svc", "ui://weather/dashboard.html"),
			"ui://svc+weather/dashboard.html"
		);
	}

	#[test]
	fn normalize_outbound_result_type_by_downstream_protocol() {
		let response = ServerJsonRpcMessage::response(
			ServerResult::ListToolsResult(
				ListToolsResult::with_all_items(vec![])
					.with_ttl_ms(30_000)
					.with_cache_scope(CacheScope::Public),
			),
			RequestId::Number(1),
		);

		let modern =
			serde_json::to_value(normalize_outbound_for_protocol(response.clone(), true)).unwrap();
		assert_eq!(modern["result"]["resultType"], "complete");
		assert_eq!(modern["result"]["ttlMs"], 30_000);
		assert_eq!(modern["result"]["cacheScope"], "public");

		let legacy = serde_json::to_value(normalize_outbound_for_protocol(response, false)).unwrap();
		assert!(legacy["result"].get("resultType").is_none());
		assert!(legacy["result"].get("ttlMs").is_none());
		assert!(legacy["result"].get("cacheScope").is_none());

		// Gateway cache hints make unstamped results stale and private for modern clients.
		// Legacy normalization omits those fields.
		let stamped = with_gateway_cache_policy(ServerJsonRpcMessage::response(
			ServerResult::ListResourcesResult(ListResourcesResult::default()),
			RequestId::Number(2),
		));
		let modern =
			serde_json::to_value(normalize_outbound_for_protocol(stamped.clone(), true)).unwrap();
		assert_eq!(modern["result"]["ttlMs"], 0);
		assert_eq!(modern["result"]["cacheScope"], "private");
		let legacy = serde_json::to_value(normalize_outbound_for_protocol(stamped, false)).unwrap();
		assert!(legacy["result"].get("ttlMs").is_none());
		assert!(legacy["result"].get("cacheScope").is_none());
	}

	#[tokio::test]
	async fn messages_to_response_captures_first_matching_tool_result() {
		let log = AsyncLog::default();
		let mut info = MCPInfo::default();
		info.set_tool("mcp".to_string(), "echo".to_string());
		log.store(Some(info));

		let stream = stream::iter(vec![
			Ok(ServerJsonRpcMessage::response(
				ServerResult::ListToolsResult(ListToolsResult {
					tools: vec![],
					..Default::default()
				}),
				RequestId::Number(1),
			)),
			Ok(ServerJsonRpcMessage::response(
				ServerResult::CallToolResult(CallToolResult::structured(json!({
					"status": "ok",
				}))),
				RequestId::Number(42),
			)),
			Ok(ServerJsonRpcMessage::error(
				ErrorData::internal_error("later error", None),
				Some(RequestId::Number(42)),
			)),
		]);

		let response =
			messages_to_response(RequestId::Number(42), stream, Some(log.clone()), false).unwrap();
		let _ = crate::http::read_resp_body(response).await.unwrap();

		let info = log.take().unwrap();
		assert_eq!(
			info.tool.as_ref().unwrap().result.as_ref().unwrap()["structuredContent"]["status"],
			"ok"
		);
		assert!(info.tool.as_ref().unwrap().error.is_none());
	}

	#[tokio::test]
	async fn messages_to_response_captures_transport_error_and_ends_stream() {
		let log = AsyncLog::default();
		let mut info = MCPInfo::default();
		info.set_tool("mcp".to_string(), "echo".to_string());
		log.store(Some(info));

		let stream = stream::iter(vec![
			Err(ClientError::new(anyhow::anyhow!("boom"))),
			Ok(ServerJsonRpcMessage::response(
				ServerResult::CallToolResult(CallToolResult::structured(json!({
					"status": "ok",
				}))),
				RequestId::Number(7),
			)),
		]);
		let messages = into_sse_stream(RequestId::Number(7), stream, Some(log.clone()), false)
			.collect::<Vec<_>>()
			.await;
		assert_eq!(messages.len(), 1, "the transport error must end the stream");
		assert!(matches!(
			messages[0].message.as_ref(),
			ServerJsonRpcMessage::Error(error) if error.id == Some(RequestId::Number(7))
		));

		let info = log.take().unwrap();
		assert!(info.tool.as_ref().unwrap().result.is_none());
		assert_eq!(
			info.tool.as_ref().unwrap().error.as_ref().unwrap()["code"],
			-32603
		);
		assert!(
			info.tool.as_ref().unwrap().error.as_ref().unwrap()["message"]
				.as_str()
				.unwrap()
				.contains("boom")
		);
	}

	#[tokio::test]
	async fn messages_to_response_captures_json_rpc_error() {
		let log = AsyncLog::default();
		let mut info = MCPInfo::default();
		info.set_tool("mcp".to_string(), "echo".to_string());
		log.store(Some(info));

		let stream = stream::iter(vec![Ok(ServerJsonRpcMessage::error(
			ErrorData::internal_error("boom", None),
			Some(RequestId::Number(7)),
		))]);
		let response =
			messages_to_response(RequestId::Number(7), stream, Some(log.clone()), false).unwrap();
		let _ = crate::http::read_resp_body(response).await.unwrap();

		let info = log.take().unwrap();
		assert!(info.tool.as_ref().unwrap().result.is_none());
		assert_eq!(
			info.tool.as_ref().unwrap().error.as_ref().unwrap()["code"],
			-32603
		);
		assert!(
			info.tool.as_ref().unwrap().error.as_ref().unwrap()["message"]
				.as_str()
				.unwrap()
				.contains("boom")
		);
	}
}
