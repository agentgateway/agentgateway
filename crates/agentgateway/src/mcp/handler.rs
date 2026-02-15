use agent_core::strng::Strng;
use agent_core::trcng;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_core::Stream;
use futures_util::StreamExt;
use http::StatusCode;
use http::request::Parts;
use opentelemetry::global::BoxedSpan;
use opentelemetry::trace::{SpanContext, SpanKind, TraceContextExt, TraceState};
use opentelemetry::{Context, TraceFlags};
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use rmcp::ErrorData;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, Implementation, JsonRpcNotification,
	JsonRpcRequest, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
	ListTasksResult, ListToolsResult, Meta, Prompt, PromptsCapability, ProtocolVersion, RequestId,
	ResourcesCapability, ServerCapabilities, ServerInfo, ServerJsonRpcMessage, ServerNotification,
	ServerResult, TasksCapability, Tool, ToolsCapability,
};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use crate::http::Response;
use crate::http::jwt::Claims;
use crate::http::sessionpersistence::MCPSession;
use crate::mcp::mergestream::{MergeFn, MessageMapper};
use crate::mcp::rbac::{CelExecWrapper, Identity, McpAuthorizationSet};
use crate::mcp::router::McpBackendGroup;
use crate::mcp::streamablehttp::ServerSseMessage;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, MCPInfo, mergestream, rbac, upstream};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::AsyncLog;
use crate::telemetry::trc::TraceParent;

// Double underscore namespacing (SEP-993) to avoid collision with standard tool underscores.
const DELIMITER: &str = "__";
const UPSTREAM_REQUEST_ID_PREFIX: &str = "agw";
const UPSTREAM_REQUEST_ID_SEPARATOR: &str = "::";
const AGW_SCHEME: &str = "agw";
const URI_PARAM: &str = "u";

fn encode_target_for_uri_host(target: &str) -> String {
	utf8_percent_encode(target, NON_ALPHANUMERIC).to_string()
}

fn decode_target_from_uri_host(target: &str) -> Option<String> {
	percent_decode_str(target)
		.decode_utf8()
		.ok()
		.map(|decoded| decoded.into_owned())
}

/// Prepends the server name to a resource name when multiplexing.
///
/// This ensures resource names are unique across multiple backends.
/// For example, `echo` becomes `serverA__echo`.
///
/// Note: We use `__` (double underscore) to align with the Envoy kNameDelimiter
/// and SEP-993 consensus for tool/prompt namespacing.
fn resource_name<'a>(
	default_target_name: Option<&str>,
	target: &str,
	name: Cow<'a, str>,
) -> Cow<'a, str> {
	if default_target_name.is_some() {
		return name;
	}
	Cow::Owned(format!("{target}{DELIMITER}{name}"))
}

fn prefix_task_id(default_target_name: Option<&str>, server_name: &str, task_id: &mut String) {
	let old_id = std::mem::take(task_id);
	*task_id = resource_name(default_target_name, server_name, Cow::Owned(old_id)).into_owned();
}

/// Wraps a resource URI in the `agw://` scheme for multiplexing.
///
/// Example: `memo://insights` from `counter` -> `agw://counter/?u=memo%3A%2F%2Finsights`
///
/// We use `agw://target/?u=uri` instead of scheme-mangling (like Envoy's `+`) to keep the
/// host fixed for security policies and to handle URI templates robustly.
///
/// Note: Braces `{}` are preserved unencoded so clients recognize URI templates.
/// These are safely decoded during the unwrap phase.
fn wrap_resource_uri<'a>(
	default_target_name: Option<&str>,
	target: &str,
	uri: &'a str,
) -> Cow<'a, str> {
	if default_target_name.is_some() {
		return Cow::Borrowed(uri);
	}

	let mut encoded = String::with_capacity(uri.len() + target.len() + 32);
	encoded.push_str(AGW_SCHEME);
	encoded.push_str("://");
	encoded.push_str(&encode_target_for_uri_host(target));
	encoded.push_str("/?");
	encoded.push_str(URI_PARAM);
	encoded.push('=');

	let bytes = uri.as_bytes();
	let mut start = 0;

	for (i, &b) in bytes.iter().enumerate() {
		if b == b'{' || b == b'}' {
			if i > start {
				for s in url::form_urlencoded::byte_serialize(&bytes[start..i]) {
					encoded.push_str(s);
				}
			}
			encoded.push(b as char);
			start = i + 1;
		}
	}
	if start < bytes.len() {
		for s in url::form_urlencoded::byte_serialize(&bytes[start..]) {
			encoded.push_str(s);
		}
	}

	Cow::Owned(encoded)
}

fn merge_meta(entries: impl IntoIterator<Item = (Strng, Option<Meta>)>) -> Option<Meta> {
	let mut items = entries
		.into_iter()
		.filter_map(|(server_name, meta)| meta.map(|m| (server_name, m)));

	let first = items.next()?;
	let Some(second) = items.next() else {
		return Some(first.1);
	};

	let mut per_upstream = Map::new();
	per_upstream.insert(first.0.to_string(), Value::Object(first.1.0));
	per_upstream.insert(second.0.to_string(), Value::Object(second.1.0));
	for (server_name, meta) in items {
		per_upstream.insert(server_name.to_string(), Value::Object(meta.0));
	}

	let mut root = Map::new();
	root.insert("upstreams".to_string(), Value::Object(per_upstream));
	Some(Meta(root))
}

fn decode_upstream_request_id_encoded(raw: &str) -> Result<(String, RequestId), UpstreamError> {
	let mut parts = raw.split(UPSTREAM_REQUEST_ID_SEPARATOR);
	let (Some(prefix), Some(encoded_server), Some(kind), Some(encoded_value), None) = (
		parts.next(),
		parts.next(),
		parts.next(),
		parts.next(),
		parts.next(),
	) else {
		return Err(UpstreamError::InvalidRequest(
			"upstream request id malformed".to_string(),
		));
	};
	if prefix != UPSTREAM_REQUEST_ID_PREFIX {
		return Err(UpstreamError::InvalidRequest(
			"upstream request id missing gateway prefix".to_string(),
		));
	}
	let server_name_bytes = URL_SAFE_NO_PAD.decode(encoded_server).map_err(|_| {
		UpstreamError::InvalidRequest("upstream request id server decode failed".to_string())
	})?;
	let server_name = String::from_utf8(server_name_bytes).map_err(|_| {
		UpstreamError::InvalidRequest("upstream request id server utf8 decode failed".to_string())
	})?;
	let value_bytes = URL_SAFE_NO_PAD.decode(encoded_value).map_err(|_| {
		UpstreamError::InvalidRequest("upstream request id value decode failed".to_string())
	})?;
	let value = String::from_utf8(value_bytes).map_err(|_| {
		UpstreamError::InvalidRequest("upstream request id value utf8 decode failed".to_string())
	})?;
	let original_id = match kind {
		"n" => value.parse::<i64>().map(RequestId::Number).map_err(|_| {
			UpstreamError::InvalidRequest("upstream request id number parse failed".to_string())
		})?,
		"s" => RequestId::String(value.into()),
		_ => {
			return Err(UpstreamError::InvalidRequest(
				"upstream request id kind unknown".to_string(),
			));
		},
	};
	Ok((server_name, original_id))
}

#[derive(Debug, Clone)]
pub struct Relay {
	upstreams: Arc<upstream::UpstreamGroup>,
	pub policies: McpAuthorizationSet,
	// If we have 1 target only, we don't prefix everything with 'target_'.
	// Else this is empty
	default_target_name: Option<String>,
	is_multiplexing: bool,
	upstream_infos: Arc<RwLock<HashMap<Strng, ServerInfo>>>,
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
			upstream_infos: Arc::new(RwLock::new(HashMap::new())),
		})
	}

	pub fn parse_resource_name<'a, 'b: 'a>(
		&'a self,
		res: &'b str,
	) -> Result<(&'a str, &'b str), UpstreamError> {
		if let Some(default) = self.default_target_name.as_deref() {
			Ok((default, res))
		} else {
			res
				.split_once(DELIMITER)
				.ok_or(UpstreamError::InvalidRequest(
					"invalid resource name".to_string(),
				))
		}
	}

	pub fn unwrap_resource_uri(&self, uri: &str) -> Option<(String, String)> {
		if let Some(default) = self.default_target_name.as_deref() {
			return Some((default.to_string(), uri.to_string()));
		}
		let parsed = url::Url::parse(uri).ok()?;
		if parsed.scheme() != AGW_SCHEME {
			return None;
		}
		let target = decode_target_from_uri_host(parsed.host_str()?)?;
		parsed
			.query_pairs()
			.find(|(k, _)| k == URI_PARAM)
			.map(|(_, v)| (target, v.into_owned()))
	}

	fn should_prefix_identifiers(&self) -> bool {
		self.default_target_name.is_none()
	}

	/// Rewrites a downstream Request ID to ensure uniqueness across upstreams.
	///
	/// Format: `agw::base64url(server_name)::kind::base64url(value)`
	/// This allows `decode_upstream_request_id` to route the response back to the correct server.
	fn encode_upstream_request_id(&self, server_name: &str, id: &RequestId) -> RequestId {
		if !self.should_prefix_identifiers() {
			return id.clone();
		}
		let (kind, value) = match id {
			RequestId::Number(n) => ("n", n.to_string()),
			RequestId::String(s) => ("s", s.to_string()),
		};
		let encoded_server = URL_SAFE_NO_PAD.encode(server_name.as_bytes());
		let encoded_value = URL_SAFE_NO_PAD.encode(value.as_bytes());
		RequestId::String(
			format!(
				"{UPSTREAM_REQUEST_ID_PREFIX}{UPSTREAM_REQUEST_ID_SEPARATOR}{encoded_server}{UPSTREAM_REQUEST_ID_SEPARATOR}{kind}{UPSTREAM_REQUEST_ID_SEPARATOR}{encoded_value}"
			)
			.into(),
		)
	}

	fn encode_upstream_progress_token(
		&self,
		server_name: &str,
		token: &rmcp::model::ProgressToken,
	) -> rmcp::model::ProgressToken {
		if !self.should_prefix_identifiers() {
			return token.clone();
		}
		rmcp::model::ProgressToken(self.encode_upstream_request_id(server_name, &token.0))
	}

	/// Decodes an upstream request ID that was previously encoded by the gateway.
	///
	/// In multiplexing mode, the gateway prefixes all request IDs with encoded upstream metadata to track
	/// which upstream server originated the request. This function reverses that process to identify the
	/// target server and the original request ID.
	///
	/// # Nuances
	/// - **Separator Collision:** Server names and IDs can contain `::`; we avoid collisions by base64url
	///   encoding metadata fields.
	/// - **Gateway Prefix:** We strictly enforce the `agw` prefix to prevent handling IDs we didn't generate.
	/// - **Single Backend Optimization:** If not multiplexing, we return the ID as-is (no allocation/parsing).
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
		decode_upstream_request_id_encoded(raw.as_ref())
	}

	/// Returns a list of upstream server names that support a specific capability.
	///
	/// Used for efficient fanout (e.g., only send `list_tools` to servers that support tools).
	///
	/// # Fallback Logic
	/// If a server has not yet completed initialization (and thus is missing from `upstream_infos`),
	/// we **include it in the list**. This "fail-open" behavior ensures that requests are not silently
	/// dropped during the startup race window. If the server genuinely doesn't support the feature,
	/// it will return a standard error which the merge logic handles.
	fn upstreams_with_capability(&self, check: impl Fn(&ServerCapabilities) -> bool) -> Vec<Strng> {
		let Ok(infos) = self.upstream_infos.read() else {
			return vec![];
		};
		self
			.upstreams
			.iter_named()
			.filter_map(|(name, _)| {
				match infos.get(&name) {
					Some(info) => check(&info.capabilities).then_some(name),
					// If we haven't received the initialize result yet, we assume the server *might* support it.
					// This ensures that if a client calls list_tools before initialize finishes, we don't silently drop it.
					None => Some(name),
				}
			})
			.collect()
	}

	pub fn upstreams_with_prompts(&self) -> Vec<Strng> {
		self.upstreams_with_capability(|caps| caps.prompts.is_some())
	}

	pub fn upstreams_with_resources(&self) -> Vec<Strng> {
		self.upstreams_with_capability(|caps| caps.resources.is_some())
	}

	pub fn upstreams_with_tasks(&self) -> Vec<Strng> {
		self.upstreams_with_capability(|caps| caps.tasks.is_some())
	}

	pub fn upstreams_with_tools(&self) -> Vec<Strng> {
		self.upstreams_with_capability(|caps| caps.tools.is_some())
	}

	pub fn upstreams_with_logging(&self) -> Vec<Strng> {
		self.upstreams_with_capability(|caps| caps.logging.is_some())
	}

	fn map_server_message(
		&self,
		server_name: &str,
		mut message: ServerJsonRpcMessage,
	) -> ServerJsonRpcMessage {
		match &mut message {
			ServerJsonRpcMessage::Request(req) => {
				req.id = self.encode_upstream_request_id(server_name, &req.id);
				if let rmcp::model::ServerRequest::CreateElicitationRequest(
					rmcp::model::CreateElicitationRequest {
						params:
							rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
								elicitation_id, ..
							},
						..
					},
				) = &req.request
				{
					tracing::debug!(
						%elicitation_id,
						"received url elicitation request"
					);
				}
			},
			ServerJsonRpcMessage::Response(resp) => {
				self.map_server_result(server_name, &mut resp.result);
			},
			ServerJsonRpcMessage::Notification(notif) => match &mut notif.notification {
				ServerNotification::ResourceUpdatedNotification(run) => {
					run.params.uri = wrap_resource_uri(
						self.default_target_name.as_deref(),
						server_name,
						&run.params.uri,
					)
					.into_owned();
				},
				ServerNotification::CancelledNotification(cn) => {
					cn.params.request_id =
						self.encode_upstream_request_id(server_name, &cn.params.request_id);
				},
				ServerNotification::ProgressNotification(pn) => {
					pn.params.progress_token =
						self.encode_upstream_progress_token(server_name, &pn.params.progress_token);
				},
				_ => {},
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
			ServerResult::ReadResourceResult(r) => {
				for content in &mut r.contents {
					let uri = match content {
						rmcp::model::ResourceContents::TextResourceContents { uri, .. } => uri,
						rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => uri,
					};
					*uri =
						wrap_resource_uri(self.default_target_name.as_deref(), server_name, uri).into_owned();
				}
			},
			ServerResult::CreateTaskResult(r) => {
				prefix_task_id(
					self.default_target_name.as_deref(),
					server_name,
					&mut r.task.task_id,
				);
			},
			ServerResult::ListTasksResult(r) => {
				for task in &mut r.tasks {
					prefix_task_id(
						self.default_target_name.as_deref(),
						server_name,
						&mut task.task_id,
					);
				}
			},
			ServerResult::GetTaskResult(r) => {
				prefix_task_id(
					self.default_target_name.as_deref(),
					server_name,
					&mut r.task.task_id,
				);
			},
			ServerResult::CancelTaskResult(r) => {
				prefix_task_id(
					self.default_target_name.as_deref(),
					server_name,
					&mut r.task.task_id,
				);
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

	fn message_mapper(&self) -> Option<MessageMapper> {
		if self.should_prefix_identifiers() {
			let relay = self.clone();
			Some(Arc::new(move |server_name: &str, message| {
				relay.map_server_message(server_name, message)
			}))
		} else {
			None
		}
	}

	pub fn merge_tools(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.default_target_name.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::with_capacity(streams.len());
			let mut tools = Vec::new();
			for (server_name, s) in streams {
				let ServerResult::ListToolsResult(ltr) = s else {
					continue;
				};
				let upstream_tools = ltr.tools;
				let meta = ltr.meta;
				meta_entries.push((server_name.clone(), meta));
				tools.reserve(upstream_tools.len());
				for t in upstream_tools {
					// Apply authorization policies, filtering tools that are not allowed.
					if !policies.validate(
						&rbac::ResourceType::Tool(rbac::ResourceId::new(
							server_name.to_string(),
							t.name.to_string(),
						)),
						&cel,
					) {
						continue;
					}
					// Rename to handle multiplexing
					tools.push(Tool {
						name: resource_name(default_target_name.as_deref(), server_name.as_str(), t.name),
						..t
					});
				}
			}
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

	pub fn merge_initialize(&self, pv: ProtocolVersion, multiplexing: bool) -> Box<MergeFn> {
		let info_store = self.upstream_infos.clone();
		Box::new(move |s| {
			if let Ok(mut infos) = info_store.write() {
				for (name, result) in &s {
					if let ServerResult::InitializeResult(info) = result {
						infos.insert(name.clone(), info.clone());
					}
				}
			}
			if !multiplexing {
				// Happy case: we can forward everything
				return match s.into_iter().next() {
					Some((_, ServerResult::InitializeResult(ir))) => Ok(ir.clone().into()),
					_ => Ok(Self::get_info(pv, multiplexing).into()),
				};
			}

			// Multiplexing is more complex. We need to find the lowest protocol version that all servers support.
			let mut has_tools = false;
			let mut has_prompts = false;
			let mut has_tasks = false;
			let mut has_resources = false;
			let mut has_resource_subscribe = false;
			let mut has_resource_list_changed = false;
			let mut has_logging = false;
			let mut has_completions = false;
			let mut extensions = std::collections::BTreeMap::new();

			let lowest_version = s
				.into_iter()
				.flat_map(|(_, v)| match v {
					ServerResult::InitializeResult(r) => {
						has_tools |= r.capabilities.tools.is_some();
						has_prompts |= r.capabilities.prompts.is_some();
						has_tasks |= r.capabilities.tasks.is_some();
						if let Some(res) = &r.capabilities.resources {
							has_resources = true;
							has_resource_subscribe |= res.subscribe.unwrap_or_default();
							has_resource_list_changed |= res.list_changed.unwrap_or_default();
						}
						has_logging |= r.capabilities.logging.is_some();
						has_completions |= r.capabilities.completions.is_some();
						if let Some(ext) = &r.capabilities.extensions {
							extensions.extend(ext.clone());
						}
						Some(r.protocol_version)
					},
					_ => None,
				})
				.min_by_key(|i| i.to_string())
				.unwrap_or(pv);
			let capabilities = ServerCapabilities {
				completions: has_completions.then_some(rmcp::model::JsonObject::default()),
				experimental: None,
				logging: has_logging.then_some(rmcp::model::JsonObject::default()),
				tasks: has_tasks.then_some(TasksCapability::default()),
				tools: has_tools.then_some(ToolsCapability::default()),
				prompts: has_prompts.then_some(PromptsCapability::default()),
				resources: has_resources.then_some(ResourcesCapability {
					subscribe: Some(has_resource_subscribe),
					list_changed: Some(has_resource_list_changed),
				}),
				extensions: if extensions.is_empty() {
					None
				} else {
					Some(extensions)
				},
			};
			let instructions = Some(
				"This server is a gateway to a set of mcp servers. It is responsible for routing requests to the correct server and aggregating the results.".to_string(),
			);
			Ok(
				ServerInfo {
					protocol_version: lowest_version,
					capabilities,
					server_info: Implementation::from_build_env(),
					instructions,
				}
				.into(),
			)
		})
	}

	pub fn merge_prompts(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.default_target_name.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::with_capacity(streams.len());
			let mut prompts = Vec::new();
			for (server_name, s) in streams {
				let ServerResult::ListPromptsResult(lpr) = s else {
					continue;
				};
				let upstream_prompts = lpr.prompts;
				let meta = lpr.meta;
				meta_entries.push((server_name.clone(), meta));
				prompts.reserve(upstream_prompts.len());
				for p in upstream_prompts {
					if !policies.validate(
						&rbac::ResourceType::Prompt(rbac::ResourceId::new(
							server_name.to_string(),
							p.name.to_string(),
						)),
						&cel,
					) {
						continue;
					}
					prompts.push(Prompt {
						name: resource_name(
							default_target_name.as_deref(),
							server_name.as_str(),
							Cow::Owned(p.name),
						)
						.into_owned(),
						..p
					});
				}
			}
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
	pub fn merge_resources(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.default_target_name.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::with_capacity(streams.len());
			let mut resources = Vec::new();
			for (server_name, s) in streams {
				let ServerResult::ListResourcesResult(lrr) = s else {
					continue;
				};
				let upstream_resources = lrr.resources;
				let meta = lrr.meta;
				meta_entries.push((server_name.clone(), meta));
				resources.reserve(upstream_resources.len());
				for mut r in upstream_resources {
					if !policies.validate(
						&rbac::ResourceType::Resource(rbac::ResourceId::new(
							server_name.to_string(),
							r.uri.to_string(),
						)),
						&cel,
					) {
						continue;
					}
					r.uri = wrap_resource_uri(default_target_name.as_deref(), server_name.as_str(), &r.uri)
						.into_owned();
					let old_name = std::mem::take(&mut r.name);
					r.name = resource_name(
						default_target_name.as_deref(),
						server_name.as_str(),
						Cow::Owned(old_name),
					)
					.into_owned();
					resources.push(r);
				}
			}
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
	pub fn merge_resource_templates(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.default_target_name.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::with_capacity(streams.len());
			let mut resource_templates = Vec::new();
			for (server_name, s) in streams {
				let ServerResult::ListResourceTemplatesResult(lrr) = s else {
					continue;
				};
				let upstream_templates = lrr.resource_templates;
				let meta = lrr.meta;
				meta_entries.push((server_name.clone(), meta));
				resource_templates.reserve(upstream_templates.len());
				for mut rt in upstream_templates {
					if !policies.validate(
						&rbac::ResourceType::Resource(rbac::ResourceId::new(
							server_name.to_string(),
							rt.uri_template.to_string(),
						)),
						&cel,
					) {
						continue;
					}
					rt.uri_template = wrap_resource_uri(
						default_target_name.as_deref(),
						server_name.as_str(),
						&rt.uri_template,
					)
					.into_owned();
					let old_name = std::mem::take(&mut rt.name);
					rt.name = resource_name(
						default_target_name.as_deref(),
						server_name.as_str(),
						Cow::Owned(old_name),
					)
					.into_owned();
					resource_templates.push(rt);
				}
			}
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
			let mut tasks = Vec::new();
			for (server_name, s) in streams {
				let ServerResult::ListTasksResult(ltr) = s else {
					continue;
				};
				let upstream_tasks = ltr.tasks;
				tasks.reserve(upstream_tasks.len());
				for mut task in upstream_tasks {
					prefix_task_id(
						default_target_name.as_deref(),
						server_name.as_str(),
						&mut task.task_id,
					);
					tasks.push(task);
				}
			}
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
			match con.get_event_stream(&ctx).await {
				Ok(s) => streams.push((name, s)),
				Err(e) => {
					tracing::debug!(
						%name,
						?e,
						"upstream failed to provide event stream; skipping for this session"
					);
				},
			}
		}

		let ms = mergestream::MergeStream::new_without_merge(streams, self.message_mapper());
		messages_to_response(RequestId::Number(0), ms)
	}
	pub async fn send_fanout(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
	) -> Result<Response, UpstreamError> {
		let id = r.id.clone();
		let mut streams = Vec::with_capacity(self.upstreams.size());
		let mut request_template = Some(r);
		let mut upstreams = self.upstreams.iter_named().peekable();
		while let Some((name, con)) = upstreams.next() {
			let request = if upstreams.peek().is_some() {
				request_template
					.as_ref()
					.ok_or_else(|| {
						UpstreamError::InvalidRequest(
							"fanout request template unexpectedly missing".to_string(),
						)
					})?
					.clone()
			} else {
				request_template.take().ok_or_else(|| {
					UpstreamError::InvalidRequest(
						"fanout request template unexpectedly missing for final upstream".to_string(),
					)
				})?
			};
			match con.generic_stream(request, &ctx).await {
				Ok(s) => streams.push((name, s)),
				Err(e) => {
					tracing::warn!(
						%name,
						?e,
						"upstream failed during fanout; excluding from results"
					);
				},
			}
		}

		if streams.is_empty() {
			return Err(UpstreamError::InvalidRequest(
				"all upstreams failed to respond to fanout".to_string(),
			));
		}

		let ms = mergestream::MergeStream::new(streams, id.clone(), merge, self.message_mapper());
		messages_to_response(id, ms)
	}

	pub async fn send_fanout_to(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
		names: Vec<Strng>,
	) -> Result<Response, UpstreamError> {
		let id = r.id.clone();
		let names_len = names.len();
		let mut streams = Vec::with_capacity(names_len);
		let mut request_template = Some(r);
		for (idx, name) in names.into_iter().enumerate() {
			let con = self
				.upstreams
				.get(name.as_ref())
				.map_err(|e| UpstreamError::InvalidRequest(e.to_string()))?;
			let request = if idx + 1 == names_len {
				request_template.take().ok_or_else(|| {
					UpstreamError::InvalidRequest(
						"targeted fanout request template unexpectedly missing for final upstream".to_string(),
					)
				})?
			} else {
				request_template
					.as_ref()
					.ok_or_else(|| {
						UpstreamError::InvalidRequest(
							"targeted fanout request template unexpectedly missing".to_string(),
						)
					})?
					.clone()
			};
			match con.generic_stream(request, &ctx).await {
				Ok(s) => streams.push((name, s)),
				Err(e) => {
					tracing::warn!(
						%name,
						?e,
						"targeted upstream failed during fanout; excluding"
					);
				},
			}
		}

		if streams.is_empty() {
			return Err(UpstreamError::InvalidRequest(
				"all targeted upstreams failed to respond to fanout".to_string(),
			));
		}

		let ms = mergestream::MergeStream::new(streams, id.clone(), merge, self.message_mapper());
		messages_to_response(id, ms)
	}
	pub async fn send_notification(
		&self,
		r: JsonRpcNotification<ClientNotification>,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		if let ClientNotification::CancelledNotification(mut cn) = r.notification {
			if let Ok((server_name, original_id)) = self.decode_upstream_request_id(&cn.params.request_id)
			{
				// Targeted routing for cancellation
				if let Ok(us) = self.upstreams.get(&server_name) {
					cn.params.request_id = original_id;
					if let Err(e) = us
						.generic_notification(ClientNotification::CancelledNotification(cn), &ctx)
						.await
					{
						tracing::warn!(%server_name, ?e, "targeted cancellation failed");
					}
					return Ok(accepted_response());
				}
			}
			// Fallback to fanout if decoding fails or server not found (e.g. not multiplexing)
			for (name, con) in self.upstreams.iter_named() {
				if let Err(e) = con
					.generic_notification(ClientNotification::CancelledNotification(cn.clone()), &ctx)
					.await
				{
					tracing::warn!(%name, ?e, "cancellation fanout failed; ignoring");
				}
			}
		} else {
			// Regular fanout for other notifications (like 'initialized')
			for (name, con) in self.upstreams.iter_named() {
				if let Err(e) = con.generic_notification(r.notification.clone(), &ctx).await {
					tracing::warn!(
						%name,
						?e,
						"upstream notification failed; ignoring"
					);
				}
			}
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
	fn get_info(pv: ProtocolVersion, _multiplexing: bool) -> ServerInfo {
		let capabilities = ServerCapabilities {
			completions: None,
			experimental: None,
			logging: None,
			tasks: Some(TasksCapability::default()),
			tools: Some(ToolsCapability::default()),
			prompts: Some(PromptsCapability::default()),
			resources: Some(ResourcesCapability::default()),
			extensions: None,
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

	let cel = CelExecWrapper::new(http);

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
	use proptest::prelude::*;
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

	#[test]
	fn wrap_resource_uri_preserves_template_braces() {
		let wrapped = wrap_resource_uri(None, "counter", "memo://{bucket}/path");
		assert!(wrapped.contains("{bucket}"));
	}

	proptest! {
		#[test]
		fn wrap_resource_uri_roundtrip(
			target in "[a-zA-Z0-9_-]{1,16}",
			uri in ".*"
		) {
			let wrapped = wrap_resource_uri(None, &target, &uri).into_owned();
			let parsed = url::Url::parse(&wrapped).expect("wrapped uri should be valid");
			prop_assert_eq!(parsed.scheme(), AGW_SCHEME);
			let decoded_target = parsed
				.host_str()
				.and_then(decode_target_from_uri_host);
			prop_assert_eq!(decoded_target.as_deref(), Some(target.as_str()));
			let extracted = parsed
				.query_pairs()
				.find(|(k, _)| k == URI_PARAM)
				.map(|(_, v)| v.into_owned())
				.expect("wrapped uri should contain u param");
			prop_assert_eq!(extracted, uri);
		}

		#[test]
		fn resource_name_prefixes_when_multiplexing(
			target in "[a-zA-Z0-9]{1,12}",
			name in "[a-zA-Z0-9]{1,12}"
		) {
			let prefixed = resource_name(None, &target, Cow::Owned(name.clone()));
			prop_assert_eq!(prefixed, format!("{target}{DELIMITER}{name}"));
		}
	}

	#[tokio::test]
	async fn decode_upstream_request_id_handles_colons_in_server_name() {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					Arc::new(crate::mcp::router::McpTarget {
						name: "server::with::colons".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
					Arc::new(crate::mcp::router::McpTarget {
						name: "other".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
				],
				stateful: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let id = relay.encode_upstream_request_id("server::with::colons", &RequestId::Number(1));
		let (decoded_name, decoded_id) = relay
			.decode_upstream_request_id(&id)
			.expect("decode failed");
		assert_eq!(decoded_name, "server::with::colons");
		assert_eq!(decoded_id, RequestId::Number(1));
	}

	#[tokio::test]
	async fn decode_upstream_request_id_roundtrip_with_separator_in_string_id() {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					Arc::new(crate::mcp::router::McpTarget {
						name: "server::with::colons".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
					Arc::new(crate::mcp::router::McpTarget {
						name: "other".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
				],
				stateful: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let original = RequestId::String("my::id:with::separators".into());
		let id = relay.encode_upstream_request_id("server::with::colons", &original);
		let (decoded_name, decoded_id) = relay
			.decode_upstream_request_id(&id)
			.expect("decode failed");
		assert_eq!(decoded_name, "server::with::colons");
		assert_eq!(decoded_id, original);
	}

	#[tokio::test]
	async fn decode_upstream_request_id_rejects_legacy_format() {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					Arc::new(crate::mcp::router::McpTarget {
						name: "serverA".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
					Arc::new(crate::mcp::router::McpTarget {
						name: "serverB".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
				],
				stateful: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let legacy = RequestId::String("agw::serverA::n:1".into());
		let err = relay
			.decode_upstream_request_id(&legacy)
			.expect_err("legacy format should be rejected");
		assert!(matches!(err, UpstreamError::InvalidRequest(_)));
	}

	#[tokio::test]
	async fn wrap_resource_uri_roundtrip_with_host_unsafe_target_name() {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					Arc::new(crate::mcp::router::McpTarget {
						name: "prod:api".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
					Arc::new(crate::mcp::router::McpTarget {
						name: "other".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
				],
				stateful: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let wrapped = wrap_resource_uri(None, "prod:api", "memo://insights").into_owned();
		assert!(wrapped.starts_with("agw://prod%3Aapi/"));
		let (target, uri) = relay
			.unwrap_resource_uri(&wrapped)
			.expect("resource uri should unwrap");
		assert_eq!(target, "prod:api");
		assert_eq!(uri, "memo://insights");
	}

	#[tokio::test]
	async fn send_notification_cancelled_should_route_to_single_upstream_and_rewrite_id() {
		use rmcp::model::{CancelledNotification, CancelledNotificationParam, JsonRpcNotification};

		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					Arc::new(crate::mcp::router::McpTarget {
						name: "serverA".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
					Arc::new(crate::mcp::router::McpTarget {
						name: "serverB".into(),
						spec: crate::types::agent::McpTargetSpec::Stdio {
							cmd: "true".into(),
							args: vec![],
							env: Default::default(),
						},
						backend: None,
						always_use_prefix: false,
						backend_policies: Default::default(),
					}),
				],
				stateful: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let rewritten_id = relay.encode_upstream_request_id("serverA", &RequestId::Number(1));
		let notification = ClientNotification::CancelledNotification(CancelledNotification {
			method: Default::default(),
			params: CancelledNotificationParam {
				request_id: rewritten_id,
				reason: Some("client cancelled".to_string()),
			},
			extensions: Default::default(),
		});

		let r = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification,
		};

		let ctx = IncomingRequestContext::empty();
		let result = relay.send_notification(r, ctx).await;
		assert!(result.is_ok());
	}
}
