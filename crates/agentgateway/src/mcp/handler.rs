use agent_core::strng::Strng;
use futures_core::Stream;
use futures_util::{StreamExt, future::join_all};
use http::StatusCode;
use http::request::Parts;
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use rmcp::ErrorData;
use rmcp::model::{
	ClientJsonRpcMessage, ClientNotification, ClientRequest, ConstString,
	ElicitationResponseNotificationMethod, ElicitationResponseNotificationParam, Implementation,
	JsonRpcNotification, JsonRpcRequest, ListPromptsResult, ListResourceTemplatesResult,
	ListResourcesResult, ListTasksResult, ListToolsResult, Meta, PromptsCapability, ProtocolVersion,
	RequestId, ResourcesCapability, ServerCapabilities, ServerInfo, ServerJsonRpcMessage,
	ServerNotification, ServerResult, TasksCapability, ToolsCapability,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::RwLock;

use crate::cel::RequestSnapshot;
use crate::http::Response;
use crate::http::sessionpersistence::{
	Encoder, MCPSnapshotMember, MCPSnapshotRouting, MCPSnapshotState,
};
use crate::mcp;
use crate::mcp::mergestream::Messages;
use crate::mcp::mergestream::{MergeFn, MessageMapper};
use crate::mcp::rbac::{CelExecWrapper, McpAuthorizationSet};
use crate::mcp::router::McpBackendGroup;
use crate::mcp::session::SessionContinuity;
use crate::mcp::streamablehttp::ServerSseMessage;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{
	ClientError, MCPInfo, local_session_binding, mergestream, rbac, session_binding_tag, upstream,
};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::{AsyncLog, SpanWriteOnDrop, SpanWriter};

// Double underscore namespacing (SEP-993) avoids collisions with tool names that include "_".
// Reference: modelcontextprotocol/modelcontextprotocol#94.
const TARGET_NAME_DELIMITER: &str = "__";
const AGW_SCHEME: &str = "agw";
const AGW_URI_QUERY_PARAM: &str = "u";
const ELICITATION_RESPONSE_METHOD: &str = ElicitationResponseNotificationMethod::VALUE;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElicitationResponseParams {
	#[serde(flatten)]
	typed: ElicitationResponseNotificationParam,
	#[serde(flatten)]
	extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ElicitationRouteId {
	#[serde(rename = "t")]
	kind: String,
	#[serde(rename = "v")]
	version: u8,
	#[serde(rename = "b")]
	session_binding: String,
	#[serde(rename = "n")]
	target: String,
	#[serde(rename = "i")]
	original_id: String,
}

impl ElicitationRouteId {
	const KIND: &str = "agw-elicitation";
	const VERSION: u8 = 1;

	fn new(session_binding: &str, target: &str, original_id: &str) -> Self {
		Self {
			kind: Self::KIND.to_string(),
			version: Self::VERSION,
			session_binding: session_binding.to_string(),
			target: target.to_string(),
			original_id: original_id.to_string(),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PendingElicitation {
	target: String,
	original_id: String,
}

impl PendingElicitation {
	fn new(target: &str, original_id: &str) -> Self {
		Self {
			target: target.to_string(),
			original_id: original_id.to_string(),
		}
	}
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum RoutedRequestIdKind {
	#[serde(rename = "n")]
	Number,
	#[serde(rename = "s")]
	String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RoutedRequestId {
	#[serde(rename = "t")]
	kind: String,
	#[serde(rename = "v")]
	version: u8,
	#[serde(rename = "b")]
	session_binding: String,
	#[serde(rename = "n")]
	target: String,
	#[serde(rename = "k")]
	original_id_kind: RoutedRequestIdKind,
	#[serde(rename = "i")]
	original_id: String,
}

impl RoutedRequestId {
	const KIND: &str = "agw-request";
	const VERSION: u8 = 1;

	fn new(session_binding: &str, target: &str, original_id: &RequestId) -> Self {
		let (original_id_kind, original_id) = match original_id {
			RequestId::Number(value) => (RoutedRequestIdKind::Number, value.to_string()),
			RequestId::String(value) => (RoutedRequestIdKind::String, value.to_string()),
		};
		Self {
			kind: Self::KIND.to_string(),
			version: Self::VERSION,
			session_binding: session_binding.to_string(),
			target: target.to_string(),
			original_id_kind,
			original_id,
		}
	}

	fn into_parts(self) -> Result<(String, RequestId), UpstreamError> {
		if self.kind != Self::KIND {
			return Err(UpstreamError::InvalidRequest(
				"unknown routed request id kind".to_string(),
			));
		}
		if self.version != Self::VERSION {
			return Err(UpstreamError::InvalidRequest(
				"unsupported routed request id version".to_string(),
			));
		}
		let original_id = match self.original_id_kind {
			RoutedRequestIdKind::Number => {
				RequestId::Number(self.original_id.parse::<i64>().map_err(|_| {
					UpstreamError::InvalidRequest("routed request id number parse failed".to_string())
				})?)
			},
			RoutedRequestIdKind::String => RequestId::String(self.original_id.into()),
		};
		Ok((self.target, original_id))
	}
}

#[derive(Debug, Clone)]
enum RoutedCustomNotification {
	ElicitationResponse {
		original: rmcp::model::CustomNotification,
		params: ElicitationResponseParams,
	},
	Other(rmcp::model::CustomNotification),
}

#[derive(Debug, Clone, Copy)]
enum FanoutMode {
	All,
	Targeted,
}

impl FanoutMode {
	fn stream_failure_log(self) -> &'static str {
		match self {
			FanoutMode::All => "upstream failed during fanout; excluding from results",
			FanoutMode::Targeted => "targeted upstream failed during fanout; excluding",
		}
	}
}

impl RoutedCustomNotification {
	fn parse(notification: rmcp::model::CustomNotification) -> Self {
		if notification.method == ELICITATION_RESPONSE_METHOD
			&& let Some(params) = notification
				.params
				.as_ref()
				.and_then(|value| serde_json::from_value::<ElicitationResponseParams>(value.clone()).ok())
		{
			return Self::ElicitationResponse {
				original: notification,
				params,
			};
		}
		Self::Other(notification)
	}
}

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
/// Note: We use `__` (double underscore) per SEP-993 for tool/prompt namespacing
/// (modelcontextprotocol/modelcontextprotocol#94).
fn resource_name<'a>(
	default_target_name: Option<&str>,
	target: &str,
	name: Cow<'a, str>,
) -> Cow<'a, str> {
	if default_target_name.is_some() {
		return name;
	}
	Cow::Owned(format!("{target}{TARGET_NAME_DELIMITER}{name}"))
}

fn prefix_task_id(default_target_name: Option<&str>, server_name: &str, task_id: &mut String) {
	let old_id = std::mem::take(task_id);
	*task_id = resource_name(default_target_name, server_name, Cow::Owned(old_id)).into_owned();
}

/// Wraps a resource URI in the `agw://` scheme for multiplexing.
///
/// Example: `memo://insights` from `counter` -> `agw://counter/?u=memo%3A%2F%2Finsights`
///
/// We use `agw://target/?u=uri` instead of scheme-mangling to keep the host fixed for
/// security policies and to handle URI templates robustly.
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
	encoded.push_str(AGW_URI_QUERY_PARAM);
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

#[derive(Debug, Clone)]
pub struct Relay {
	upstreams: Arc<upstream::UpstreamGroup>,
	pub policies: McpAuthorizationSet,
	// If we have 1 target only, we don't prefix everything with 'target_'.
	// Else this is empty
	default_target_name: Option<String>,
	is_multiplexing: bool,
	allow_degraded: bool,
	allow_insecure_multiplex: bool,
	// std::sync::RwLock is intentional: all accesses are synchronous (inside MergeFn or
	// upstreams_with_capability), never held across await points.
	upstream_infos: Arc<RwLock<HashMap<Strng, ServerInfo>>>,
	route_id_encoder: Encoder,
	session_binding: Arc<RwLock<String>>,
	pending_elicitations: Arc<RwLock<HashMap<String, PendingElicitation>>>,
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

	pub fn build_snapshot_connections(
		self,
		members: &[MCPSnapshotMember],
		routing: MCPSnapshotRouting,
	) -> Result<Option<Relay>, mcp::Error> {
		if members.is_empty() {
			return Ok(None);
		}
		// Resume rebuilds the exact initialized subset captured in the session
		// token. We never reinterpret an existing session against whatever targets
		// happen to be healthy now.
		let matches_snapshot = match self.backend.matches_snapshot_members(members) {
			Ok(matches_snapshot) => matches_snapshot,
			Err(error) => {
				tracing::warn!(%error, "failed to fingerprint snapshot members during resume");
				return Ok(None);
			},
		};
		if !matches_snapshot {
			return Ok(None);
		}
		let runtime_allow_degraded = self.backend.allow_degraded;
		let Some(backend) = self
			.backend
			.snapshot_subset(members.iter().map(|member| member.target.as_str()), false)
		else {
			return Ok(None);
		};
		Relay::new_with_routing(
			backend,
			self.policies,
			self.client,
			runtime_allow_degraded,
			routing,
		)
		.map(Some)
	}
}

impl Relay {
	pub fn new(
		backend: McpBackendGroup,
		policies: McpAuthorizationSet,
		client: PolicyClient,
	) -> Result<Self, mcp::Error> {
		let mut is_multiplexing = false;
		let default_target_name = if backend.targets.len() != 1 {
			is_multiplexing = true;
			None
		} else if backend.targets[0].always_use_prefix {
			None
		} else {
			Some(backend.targets[0].name.to_string())
		};
		let runtime_allow_degraded = backend.allow_degraded;
		Self::new_with_routing(
			backend,
			policies,
			client,
			runtime_allow_degraded,
			MCPSnapshotRouting {
				default_target_name,
				is_multiplexing,
			},
		)
	}

	pub fn new_with_routing(
		backend: McpBackendGroup,
		policies: McpAuthorizationSet,
		client: PolicyClient,
		runtime_allow_degraded: bool,
		routing: MCPSnapshotRouting,
	) -> Result<Self, mcp::Error> {
		let allow_insecure_multiplex = backend.allow_insecure_multiplex;
		for target in &backend.targets {
			if target.name.contains(TARGET_NAME_DELIMITER) {
				return Err(mcp::Error::SendError(
					None,
					format!(
						"backend target name {:?} must not contain the reserved delimiter {:?}",
						target.name.as_str(),
						TARGET_NAME_DELIMITER
					),
				));
			}
		}
		Ok(Self {
			upstreams: Arc::new(upstream::UpstreamGroup::new(client, backend)?),
			policies,
			default_target_name: routing.default_target_name,
			is_multiplexing: routing.is_multiplexing,
			allow_degraded: runtime_allow_degraded,
			allow_insecure_multiplex,
			upstream_infos: Arc::new(RwLock::new(HashMap::new())),
			// The concrete encoder is bound once the downstream session id exists.
			route_id_encoder: Encoder::base64(),
			session_binding: Arc::new(RwLock::new(local_session_binding())),
			pending_elicitations: Arc::new(RwLock::new(HashMap::new())),
		})
	}

	pub fn with_policies(&self, policies: McpAuthorizationSet) -> Self {
		Self {
			upstreams: self.upstreams.clone(),
			policies,
			default_target_name: self.default_target_name.clone(),
			is_multiplexing: self.is_multiplexing,
			allow_degraded: self.allow_degraded,
			allow_insecure_multiplex: self.allow_insecure_multiplex,
			upstream_infos: self.upstream_infos.clone(),
			route_id_encoder: self.route_id_encoder.clone(),
			session_binding: self.session_binding.clone(),
			pending_elicitations: self.pending_elicitations.clone(),
		}
	}

	pub fn with_session_binding(mut self, session_handle: &str, route_id_encoder: Encoder) -> Self {
		self.route_id_encoder = route_id_encoder;
		self.set_session_binding(session_handle);
		self
	}

	pub fn set_session_binding(&self, session_handle: &str) {
		let mut binding = self.session_binding.write().unwrap_or_else(|e| {
			tracing::error!("session binding lock poisoned while updating; continuing");
			e.into_inner()
		});
		*binding = session_binding_tag(session_handle);
	}

	pub fn parse_resource_name<'a, 'b: 'a>(
		&'a self,
		res: &'b str,
	) -> Result<(&'a str, &'b str), UpstreamError> {
		if let Some(default) = self.default_target_name.as_deref() {
			Ok((default, res))
		} else {
			res
				.split_once(TARGET_NAME_DELIMITER)
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
			.find(|(k, _)| k == AGW_URI_QUERY_PARAM)
			.map(|(_, v)| (target, v.into_owned()))
	}

	pub fn uses_routed_identifiers(&self) -> bool {
		self.default_target_name.is_none()
	}

	fn register_pending_elicitation(&self, routed_id: &str, target: &str, original_id: &str) {
		let mut pending = self.pending_elicitations.write().unwrap_or_else(|e| {
			tracing::error!("pending elicitation lock poisoned while registering; continuing");
			e.into_inner()
		});
		pending.insert(
			routed_id.to_string(),
			PendingElicitation::new(target, original_id),
		);
	}

	fn clear_pending_elicitation(&self, target: &str, original_id: &str) {
		let needle = PendingElicitation::new(target, original_id);
		let mut pending = self.pending_elicitations.write().unwrap_or_else(|e| {
			tracing::error!("pending elicitation lock poisoned while clearing; continuing");
			e.into_inner()
		});
		let stale = pending
			.iter()
			.filter_map(|(routed_id, active)| (active == &needle).then_some(routed_id.clone()))
			.collect::<HashSet<_>>();
		if stale.is_empty() {
			return;
		}
		pending.retain(|routed_id, _| !stale.contains(routed_id));
	}

	fn take_active_elicitation(&self, routed_id: &str, target: &str, original_id: &str) -> bool {
		let expected = PendingElicitation::new(target, original_id);
		let mut pending = self.pending_elicitations.write().unwrap_or_else(|e| {
			tracing::error!("pending elicitation lock poisoned while consuming; continuing");
			e.into_inner()
		});
		pending
			.remove(routed_id)
			.is_some_and(|active| active == expected)
	}

	/// Rewrites a downstream Request ID to ensure uniqueness across upstreams.
	///
	fn encode_upstream_request_id(
		&self,
		server_name: &str,
		id: &RequestId,
	) -> Result<RequestId, ClientError> {
		if !self.uses_routed_identifiers() {
			return Ok(id.clone());
		}
		// Bind routed IDs to the current downstream session so replayed or
		// cross-session tokens cannot steer a request to the wrong upstream.
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("session binding lock poisoned while encoding request id; continuing");
			e.into_inner()
		});
		let route_id = RoutedRequestId::new(session_binding.as_str(), server_name, id);
		let plaintext =
			serde_json::to_string(&route_id).expect("serializing routed request id should not fail");
		self
			.route_id_encoder
			.encrypt(&plaintext)
			.map(|encoded| RequestId::String(encoded.into()))
			.map_err(|error| {
				ClientError::new(anyhow::anyhow!(
					"failed to encode routed request id for {server_name}: {error}"
				))
			})
	}

	fn encode_upstream_progress_token(
		&self,
		server_name: &str,
		token: &rmcp::model::ProgressToken,
	) -> Result<rmcp::model::ProgressToken, ClientError> {
		if !self.uses_routed_identifiers() {
			return Ok(token.clone());
		}
		self
			.encode_upstream_request_id(server_name, &token.0)
			.map(rmcp::model::ProgressToken)
	}

	fn encode_upstream_elicitation_id(
		&self,
		server_name: &str,
		elicitation_id: &str,
	) -> Result<String, ClientError> {
		if !self.uses_routed_identifiers() {
			return Ok(elicitation_id.to_string());
		}
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("session binding lock poisoned while encoding elicitation id; continuing");
			e.into_inner()
		});
		let route_id = ElicitationRouteId::new(session_binding.as_str(), server_name, elicitation_id);
		let plaintext =
			serde_json::to_string(&route_id).expect("serializing elicitation route id should not fail");
		self.route_id_encoder.encrypt(&plaintext).map_err(|error| {
			ClientError::new(anyhow::anyhow!(
				"failed to encode elicitation route id for {server_name}: {error}"
			))
		})
	}

	fn decode_upstream_progress_token(
		&self,
		token: &rmcp::model::ProgressToken,
	) -> Result<(String, rmcp::model::ProgressToken), UpstreamError> {
		let (server_name, original_id) = self.decode_upstream_request_id(&token.0)?;
		Ok((server_name, rmcp::model::ProgressToken(original_id)))
	}

	fn decode_upstream_elicitation_id(
		&self,
		elicitation_id: &str,
	) -> Result<(String, String), UpstreamError> {
		if let Some(default) = self.default_target_name.as_deref() {
			return Ok((default.to_string(), elicitation_id.to_string()));
		}
		// Elicitation responses may arrive after a resume, so the ID has to carry
		// its own routing authority instead of depending on local pending state.
		let encoded = self
			.route_id_encoder
			.decrypt(elicitation_id)
			.map_err(|_| UpstreamError::InvalidRequest("invalid elicitation route id".to_string()))?;
		let route_id = serde_json::from_slice::<ElicitationRouteId>(&encoded).map_err(|_| {
			UpstreamError::InvalidRequest("invalid elicitation route id payload".to_string())
		})?;
		if route_id.kind != ElicitationRouteId::KIND {
			return Err(UpstreamError::InvalidRequest(
				"unknown elicitation route id kind".to_string(),
			));
		}
		if route_id.version != ElicitationRouteId::VERSION {
			return Err(UpstreamError::InvalidRequest(
				"unsupported elicitation route id version".to_string(),
			));
		}
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("session binding lock poisoned while decoding elicitation id; continuing");
			e.into_inner()
		});
		if route_id.session_binding != session_binding.as_str() {
			return Err(UpstreamError::InvalidRequest(
				"elicitation route id does not match this session".to_string(),
			));
		}
		Ok((route_id.target, route_id.original_id))
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
		let encoded = self
			.route_id_encoder
			.decrypt(raw.as_ref())
			.map_err(|_| UpstreamError::InvalidRequest("invalid routed request id".to_string()))?;
		let route_id = serde_json::from_slice::<RoutedRequestId>(&encoded).map_err(|_| {
			UpstreamError::InvalidRequest("invalid routed request id payload".to_string())
		})?;
		let session_binding = self.session_binding.read().unwrap_or_else(|e| {
			tracing::error!("session binding lock poisoned while decoding request id; continuing");
			e.into_inner()
		});
		if route_id.session_binding != session_binding.as_str() {
			return Err(UpstreamError::InvalidRequest(
				"routed request id does not match this session".to_string(),
			));
		}
		route_id.into_parts()
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
		let infos = self.upstream_infos.read().unwrap_or_else(|e| {
			tracing::error!(
				"upstream capability cache lock poisoned; continuing with last known capabilities"
			);
			e.into_inner()
		});
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
	) -> Result<ServerJsonRpcMessage, ClientError> {
		match &mut message {
			ServerJsonRpcMessage::Request(req) => {
				req.id = self.encode_upstream_request_id(server_name, &req.id)?;
				if let rmcp::model::ServerRequest::CreateElicitationRequest(
					rmcp::model::CreateElicitationRequest {
						params:
							rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
								elicitation_id, ..
							},
						..
					},
				) = &mut req.request
				{
					let original_elicitation_id = elicitation_id.clone();
					let encoded_id =
						self.encode_upstream_elicitation_id(server_name, original_elicitation_id.as_str())?;
					self.register_pending_elicitation(&encoded_id, server_name, &original_elicitation_id);
					*elicitation_id = encoded_id;
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
						self.encode_upstream_request_id(server_name, &cn.params.request_id)?;
				},
				ServerNotification::ProgressNotification(pn) => {
					pn.params.progress_token =
						self.encode_upstream_progress_token(server_name, &pn.params.progress_token)?;
				},
				ServerNotification::ElicitationCompletionNotification(ec) => {
					self.clear_pending_elicitation(server_name, &ec.params.elicitation_id);
					ec.params.elicitation_id =
						self.encode_upstream_elicitation_id(server_name, &ec.params.elicitation_id)?;
				},
				_ => {},
			},
			_ => {},
		}
		Ok(message)
	}

	/// Rewrites identifiers embedded in single-target response payloads.
	///
	/// This only handles result types returned by single-target operations (e.g., `ReadResource`,
	/// task operations). List result types (`ListToolsResult`, `ListPromptsResult`,
	/// `ListResourcesResult`, etc.) are intentionally absent here — they are always fanout
	/// operations whose identifiers are rewritten in their respective `merge_*` functions.
	fn map_server_result(&self, server_name: &str, result: &mut ServerResult) {
		if !self.uses_routed_identifiers() {
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
	pub fn snapshot_state(&self) -> Result<MCPSnapshotState, mcp::Error> {
		let infos = self.upstream_infos.read().unwrap_or_else(|e| {
			tracing::error!(
				"upstream capability cache lock poisoned while snapshotting initialize state; continuing"
			);
			e.into_inner()
		});
		let mut members = Vec::with_capacity(infos.len());
		for (name, us) in self.upstreams.iter_named() {
			let Some(info) = infos.get(&name).cloned() else {
				continue;
			};
			let target = self.upstreams.target(name.as_str()).ok_or_else(|| {
				mcp::Error::SendError(
					None,
					format!("missing configured target for upstream {}", name.as_str()),
				)
			})?;
			let fingerprint = target.snapshot_fingerprint().map_err(|e| {
				mcp::Error::SendError(
					None,
					format!(
						"failed to fingerprint upstream {} for session snapshot: {e}",
						name.as_str()
					),
				)
			})?;
			let session = us.get_session_state().ok_or_else(|| {
				mcp::Error::SendError(
					None,
					format!(
						"upstream {} does not support resumable session snapshots",
						name.as_str()
					),
				)
			})?;
			members.push(MCPSnapshotMember::new(
				name.to_string(),
				session,
				info,
				fingerprint,
			));
		}
		if members.is_empty() {
			return Err(mcp::Error::SendError(
				None,
				"no initialized upstreams available for session snapshot".to_string(),
			));
		}
		Ok(MCPSnapshotState::new(
			members,
			MCPSnapshotRouting {
				default_target_name: self.default_target_name.clone(),
				is_multiplexing: self.is_multiplexing,
			},
		))
	}

	pub fn restore_snapshot_state(&self, members: &[MCPSnapshotMember]) -> Result<(), mcp::Error> {
		let mut restored_infos = HashMap::with_capacity(members.len());
		for member in members {
			let us = self.upstreams.get(member.target.as_str()).map_err(|_| {
				mcp::Error::SendError(
					None,
					format!(
						"snapshot target {:?} is no longer initialized",
						member.target
					),
				)
			})?;
			// Restore both upstream session affinity and the initialize-time
			// capability cache so post-resume routing matches the original session.
			us.set_session_id(member.session.as_deref(), member.backend);
			restored_infos.insert(member.target.as_str().into(), member.info.clone());
		}
		let mut infos = self.upstream_infos.write().unwrap_or_else(|e| {
			tracing::error!(
				"upstream capability cache lock poisoned while restoring initialize state; continuing"
			);
			e.into_inner()
		});
		*infos = restored_infos;
		Ok(())
	}

	pub fn count(&self) -> usize {
		self.upstreams.size()
	}

	pub fn is_multiplexing(&self) -> bool {
		self.is_multiplexing
	}

	pub fn allow_insecure_multiplex(&self) -> bool {
		self.allow_insecure_multiplex
	}

	pub fn session_continuity(&self) -> SessionContinuity {
		self.upstreams.session_continuity()
	}

	fn message_mapper(&self) -> Option<MessageMapper> {
		if self.uses_routed_identifiers() {
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
				for mut t in upstream_tools {
					// Apply authorization policies, filtering tools that are not allowed.
					if !policies.validate(
						&rbac::ResourceType::Tool(rbac::ResourceId::new(
							server_name.as_str(),
							t.name.to_string(),
						)),
						&cel,
					) {
						continue;
					}
					// Rename to handle multiplexing
					t.name = resource_name(default_target_name.as_deref(), server_name.as_str(), t.name);
					tools.push(t);
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

	pub fn merge_initialize(&self, pv: ProtocolVersion) -> Box<MergeFn> {
		let info_store = self.upstream_infos.clone();
		let multiplexing = self.is_multiplexing;
		Box::new(move |s| {
			let mut infos = info_store.write().unwrap_or_else(|e| {
				tracing::error!(
					"upstream capability cache lock poisoned while updating initialize results; continuing"
				);
				e.into_inner()
			});
			for (name, result) in &s {
				if let ServerResult::InitializeResult(info) = result {
					infos.insert(name.clone(), info.clone());
				}
			}
			if !multiplexing {
				// Happy case: we can forward everything
				return match s.into_iter().next() {
					Some((_, ServerResult::InitializeResult(ir))) => Ok(ir.into()),
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
				.min_by(|a, b| {
					a.partial_cmp(b)
						.expect("ProtocolVersion ordering must be total")
				})
				.unwrap_or(pv);
			let mut capabilities = ServerCapabilities::default();
			capabilities.completions = has_completions.then_some(rmcp::model::JsonObject::default());
			capabilities.logging = has_logging.then_some(rmcp::model::JsonObject::default());
			capabilities.tasks = has_tasks.then_some(TasksCapability::default());
			capabilities.tools = has_tools.then_some(ToolsCapability::default());
			capabilities.prompts = has_prompts.then_some(PromptsCapability::default());
			capabilities.resources = has_resources.then_some(ResourcesCapability {
				subscribe: Some(has_resource_subscribe),
				list_changed: Some(has_resource_list_changed),
			});
			capabilities.extensions = if extensions.is_empty() {
				None
			} else {
				Some(extensions)
			};
			Ok(
					ServerInfo::new(capabilities)
						.with_protocol_version(lowest_version)
						.with_server_info(Implementation::from_build_env())
						.with_instructions(
							"This server is a gateway to a set of mcp servers. It is responsible for routing requests to the correct server and aggregating the results.",
						)
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
				for mut p in upstream_prompts {
					if !policies.validate(
						&rbac::ResourceType::Prompt(rbac::ResourceId::new(
							server_name.as_str(),
							p.name.as_str(),
						)),
						&cel,
					) {
						continue;
					}
					let old_name = std::mem::take(&mut p.name);
					p.name = resource_name(
						default_target_name.as_deref(),
						server_name.as_str(),
						Cow::Owned(old_name),
					)
					.into_owned();
					prompts.push(p);
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
							server_name.as_str(),
							r.uri.as_str(),
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
							server_name.as_str(),
							rt.uri_template.as_str(),
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
	pub fn merge_tasks(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let default_target_name = self.default_target_name.clone();
		Box::new(move |streams| {
			let mut tasks = Vec::new();
			let mut upstream_result_count = 0usize;
			let mut single_next_cursor = None;
			let mut single_total = None;
			for (server_name, s) in streams {
				let ServerResult::ListTasksResult(ltr) = s else {
					continue;
				};
				upstream_result_count += 1;
				let rmcp::model::ListTasksResult {
					tasks: upstream_tasks,
					next_cursor,
					total,
					..
				} = ltr;
				if upstream_result_count == 1 {
					single_next_cursor = next_cursor;
					single_total = total;
				}
				tasks.reserve(upstream_tasks.len());
				for mut task in upstream_tasks {
					if !policies.validate(
						&rbac::ResourceType::Task(rbac::ResourceId::new(
							server_name.as_str(),
							task.task_id.as_str(),
						)),
						&cel,
					) {
						continue;
					}
					prefix_task_id(
						default_target_name.as_deref(),
						server_name.as_str(),
						&mut task.task_id,
					);
					tasks.push(task);
				}
			}
			let (next_cursor, total) = if upstream_result_count == 1 {
				(single_next_cursor, single_total)
			} else {
				// Multiplexed tasks/list stays intentionally unpaginated until the
				// gateway has a real federated cursor design.
				(None, None)
			};
			let mut out = ListTasksResult::new(tasks);
			out.next_cursor = next_cursor;
			out.total = total;
			Ok(out.into())
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
			.map(move |msg| msg.and_then(|msg| relay.map_server_message(&server_name, msg)));

		messages_to_response(id, stream)
	}
	pub async fn send_fanout_deletion(
		&self,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		let allow_degraded = self.allow_degraded;
		for (name, con) in self.upstreams.iter_named() {
			if let Err(e) = con.delete(&ctx).await {
				if !allow_degraded {
					return Err(e);
				}
				tracing::warn!(
					%name,
					?e,
					"upstream failed during session deletion; continuing cleanup"
				);
			}
		}
		Ok(accepted_response())
	}
	pub async fn get_event_stream_messages(
		&self,
		ctx: IncomingRequestContext,
	) -> Result<Messages, UpstreamError> {
		let mut streams = Vec::new();
		let allow_degraded = self.allow_degraded;
		for (name, con) in self.upstreams.iter_named() {
			if allow_degraded {
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
			} else {
				streams.push((name, con.get_event_stream(&ctx).await?));
			}
		}

		if streams.is_empty() {
			return Err(UpstreamError::InvalidRequest(
				"all upstreams failed to provide event stream".to_string(),
			));
		}

		let ms = mergestream::MergeStream::new_without_merge(streams, self.message_mapper());
		Ok(Messages::from_stream(ms))
	}

	async fn build_fanout_streams(
		&self,
		request: JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
		names: Vec<Strng>,
		mode: FanoutMode,
	) -> Result<Vec<(Strng, mergestream::Messages)>, UpstreamError> {
		let allow_degraded = self.allow_degraded;
		let ctx = ctx.clone();
		let futures = names
			.into_iter()
			.map(|name| {
				let con = self
					.upstreams
					.get(name.as_ref())
					.map_err(|e| UpstreamError::InvalidRequest(e.to_string()))?;
				let request = request.clone();
				let ctx = ctx.clone();
				Ok(async move { (name, con.generic_stream(request, &ctx).await) })
			})
			.collect::<Result<Vec<_>, UpstreamError>>()?;
		let mut streams = Vec::with_capacity(futures.len());
		for (name, result) in join_all(futures).await {
			match result {
				Ok(stream) => streams.push((name, stream)),
				Err(e) => {
					if !allow_degraded {
						return Err(e);
					}
					tracing::warn!(%name, ?e, "{}", mode.stream_failure_log());
				},
			}
		}
		Ok(streams)
	}

	async fn prepare_selected_fanout(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
		names: Vec<Strng>,
		fanout_mode: FanoutMode,
		empty_error: String,
	) -> Result<(RequestId, mergestream::MergeStream), UpstreamError> {
		let id = r.id.clone();
		let streams = self
			.build_fanout_streams(r, &ctx, names, fanout_mode)
			.await?;

		if streams.is_empty() {
			return Err(UpstreamError::InvalidRequest(empty_error));
		}

		let ms = mergestream::MergeStream::new(
			streams,
			id.clone(),
			merge,
			!self.allow_degraded,
			self.message_mapper(),
		);
		Ok((id, ms))
	}

	pub async fn send_fanout(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
	) -> Result<Response, UpstreamError> {
		let names = self
			.upstreams
			.iter_named()
			.map(|(name, _)| name)
			.collect::<Vec<_>>();
		let (id, stream) = self
			.prepare_selected_fanout(
				r,
				ctx,
				merge,
				names,
				FanoutMode::All,
				"all upstreams failed to respond to fanout".to_string(),
			)
			.await?;
		messages_to_response(id, stream)
	}

	pub async fn send_initialize(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		pv: ProtocolVersion,
	) -> Result<Response, UpstreamError> {
		let names = self
			.upstreams
			.iter_named()
			.map(|(name, _)| name)
			.collect::<Vec<_>>();
		let (id, stream) = self
			.prepare_selected_fanout(
				r,
				ctx,
				self.merge_initialize(pv),
				names,
				FanoutMode::All,
				"all upstreams failed to respond to fanout".to_string(),
			)
			.await?;
		messages_to_buffered_response(id, stream).await
	}

	pub async fn send_fanout_to(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
		names: Vec<Strng>,
	) -> Result<Response, UpstreamError> {
		let method = r.request.method().to_string();
		if names.is_empty() {
			return Err(UpstreamError::InvalidMethod(format!(
				"no eligible backends for method {method}",
			)));
		}
		let (id, stream) = self
			.prepare_selected_fanout(
				r,
				ctx,
				merge,
				names,
				FanoutMode::Targeted,
				format!("all eligible backends failed for method {method}"),
			)
			.await?;
		messages_to_response(id, stream)
	}
	pub async fn send_notification(
		&self,
		r: JsonRpcNotification<ClientNotification>,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		match r.notification {
			ClientNotification::CancelledNotification(mut cn) => {
				if let Ok((server_name, original_id)) =
					self.decode_upstream_request_id(&cn.params.request_id)
				{
					// Targeted routing for cancellation
					if let Ok(us) = self.upstreams.get(&server_name) {
						cn.params.request_id = original_id;
						if let Err(e) = us
							.generic_notification(ClientNotification::CancelledNotification(cn), &ctx)
							.await
						{
							if !self.allow_degraded {
								return Err(e);
							}
							tracing::warn!(%server_name, ?e, "targeted cancellation failed");
						}
						return Ok(accepted_response());
					}
					tracing::warn!(
						%server_name,
						request_id = %cn.params.request_id,
						"dropping cancellation: upstream not found"
					);
					return Ok(accepted_response());
				}
				if self.uses_routed_identifiers() {
					tracing::warn!(
						request_id = %cn.params.request_id,
						"dropping cancellation: failed to decode routed request id"
					);
					return Ok(accepted_response());
				}
				// Fallback to fanout only when the session is not multiplexing.
				for (name, con) in self.upstreams.iter_named() {
					if let Err(e) = con
						.generic_notification(ClientNotification::CancelledNotification(cn.clone()), &ctx)
						.await
					{
						if !self.allow_degraded {
							return Err(e);
						}
						tracing::warn!(%name, ?e, "cancellation fanout failed; ignoring");
					}
				}
			},
			ClientNotification::ProgressNotification(mut pn) => {
				if let Ok((server_name, original_progress_token)) =
					self.decode_upstream_progress_token(&pn.params.progress_token)
				{
					// Targeted routing for progress updates
					if let Ok(us) = self.upstreams.get(&server_name) {
						pn.params.progress_token = original_progress_token;
						if let Err(e) = us
							.generic_notification(ClientNotification::ProgressNotification(pn), &ctx)
							.await
						{
							if !self.allow_degraded {
								return Err(e);
							}
							tracing::warn!(%server_name, ?e, "targeted progress notification failed");
						}
						return Ok(accepted_response());
					}
					tracing::warn!(
						%server_name,
						progress_token = %pn.params.progress_token.0,
						"dropping progress notification: upstream not found"
					);
					return Ok(accepted_response());
				}
				if self.uses_routed_identifiers() {
					tracing::warn!(
						progress_token = %pn.params.progress_token.0,
						"dropping progress notification: failed to decode routed progress token"
					);
					return Ok(accepted_response());
				}
				// Fallback to fanout only when the session is not multiplexing.
				for (name, con) in self.upstreams.iter_named() {
					if let Err(e) = con
						.generic_notification(ClientNotification::ProgressNotification(pn.clone()), &ctx)
						.await
					{
						if !self.allow_degraded {
							return Err(e);
						}
						tracing::warn!(%name, ?e, "progress notification fanout failed; ignoring");
					}
				}
			},
			ClientNotification::CustomNotification(cn) => {
				let fallback_notification = match RoutedCustomNotification::parse(cn) {
					RoutedCustomNotification::ElicitationResponse {
						mut original,
						mut params,
					} => {
						let routed_elicitation_id = params.typed.elicitation_id.clone();
						if let Ok((server_name, original_elicitation_id)) =
							self.decode_upstream_elicitation_id(&routed_elicitation_id)
						{
							if !self.take_active_elicitation(
								&routed_elicitation_id,
								&server_name,
								&original_elicitation_id,
							) {
								tracing::warn!(
									%server_name,
									elicitation_id = %original_elicitation_id,
									"dropping elicitation response: no active gateway-issued elicitation"
								);
								return Ok(accepted_response());
							}
							params.typed.elicitation_id = original_elicitation_id;
							let params_value = match serde_json::to_value(&params) {
								Ok(v) => v,
								Err(e) => {
									tracing::warn!(
										%server_name,
										elicitation_id = %params.typed.elicitation_id,
										error = %e,
										"dropping elicitation response: failed to serialize params"
									);
									return Ok(accepted_response());
								},
							};
							original.params = Some(params_value);
							match self.upstreams.get(&server_name) {
								Ok(us) => {
									if let Err(e) = us
										.generic_notification(ClientNotification::CustomNotification(original), &ctx)
										.await
									{
										if !self.allow_degraded {
											return Err(e);
										}
										tracing::warn!(%server_name, ?e, "targeted elicitation response failed");
									}
								},
								Err(_) => {
									tracing::warn!(
										%server_name,
										elicitation_id = %params.typed.elicitation_id,
										"dropping elicitation response: upstream not found"
									);
								},
							}
							return Ok(accepted_response());
						}
						tracing::warn!(
							elicitation_id = %params.typed.elicitation_id,
							"dropping elicitation response: failed to decode upstream id"
						);
						return Ok(accepted_response());
					},
					RoutedCustomNotification::Other(notification) => notification,
				};

				for (name, con) in self.upstreams.iter_named() {
					if let Err(e) = con
						.generic_notification(
							ClientNotification::CustomNotification(fallback_notification.clone()),
							&ctx,
						)
						.await
					{
						if !self.allow_degraded {
							return Err(e);
						}
						tracing::warn!(
								%name,
								?e,
							"custom notification fanout failed; ignoring"
						);
					}
				}
			},
			notification => {
				// Regular fanout for other notifications (like 'initialized')
				for (name, con) in self.upstreams.iter_named() {
					if let Err(e) = con.generic_notification(notification.clone(), &ctx).await {
						if !self.allow_degraded {
							return Err(e);
						}
						tracing::warn!(
								%name,
								?e,
							"upstream notification failed; ignoring"
						);
					}
				}
			},
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
		let capabilities = ServerCapabilities::builder()
			.enable_tasks_with(TasksCapability::default())
			.enable_tools_with(ToolsCapability::default())
			.enable_prompts_with(PromptsCapability::default())
			.enable_resources_with(ResourcesCapability::default())
			.build();
		ServerInfo::new(capabilities)
				.with_protocol_version(pv)
				.with_server_info(Implementation::from_build_env())
				.with_instructions(
					"This server is a gateway to a set of mcp servers. It is responsible for routing requests to the correct server and aggregating the results.",
				)
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

	let snap = http
		.extensions
		.get::<Arc<Option<RequestSnapshot>>>()
		.cloned()
		.unwrap_or_else(|| Arc::new(None));

	let cel = CelExecWrapper::new(snap);

	let tracer = http
		.extensions
		.get::<SpanWriter>()
		.cloned()
		.unwrap_or_default();
	let _span = tracer.start(span_name.to_string());
	(_span, log, cel)
}

fn messages_to_response(
	id: RequestId,
	stream: impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
) -> Result<Response, UpstreamError> {
	use futures_util::StreamExt;
	use rmcp::model::ServerJsonRpcMessage;
	// POST response streams are request-scoped. Only the session GET stream
	// emits replayable event IDs for Last-Event-ID.
	let stream = stream.map(move |rpc| {
		let r = match rpc {
			Ok(rpc) => rpc,
			Err(e) => {
				ServerJsonRpcMessage::error(ErrorData::internal_error(e.to_string(), None), id.clone())
			},
		};
		ServerSseMessage {
			event_id: None,
			message: Arc::new(r),
		}
	});
	Ok(crate::mcp::session::sse_stream_response(stream, None))
}

async fn messages_to_buffered_response(
	id: RequestId,
	mut stream: impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Unpin,
) -> Result<Response, UpstreamError> {
	let mut messages = Vec::new();
	while let Some(rpc) = stream.next().await {
		let rpc = match rpc {
			Ok(rpc) => rpc,
			Err(e) => {
				ServerJsonRpcMessage::error(ErrorData::internal_error(e.to_string(), None), id.clone())
			},
		};
		messages.push(rpc);
	}

	let Some(first) = messages.first() else {
		return Err(UpstreamError::InvalidRequest(
			"all upstream streams ended before terminal response".to_string(),
		));
	};

	if messages.len() == 1
		&& matches!(
			first,
			ServerJsonRpcMessage::Response(_) | ServerJsonRpcMessage::Error(_)
		) {
		let body = serde_json::to_vec(first).map_err(|e| {
			UpstreamError::InvalidRequest(format!("failed to serialize jsonrpc response: {e}"))
		})?;
		return ::http::Response::builder()
			.status(StatusCode::OK)
			.header(
				http::header::CONTENT_TYPE,
				rmcp::transport::common::http_header::JSON_MIME_TYPE,
			)
			.body(crate::http::Body::from(body))
			.map_err(|e| UpstreamError::InvalidRequest(format!("failed to build json response: {e}")));
	}

	// Buffered POST responses follow the same request-scoped SSE contract.
	let stream = futures_util::stream::iter(messages.into_iter().map(|message| ServerSseMessage {
		event_id: None,
		message: Arc::new(message),
	}));
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
	use rstest::rstest;
	use serde_json::json;
	use std::collections::HashMap;
	use std::path::Path;
	use std::time::Duration;
	use tempfile::NamedTempFile;

	fn capture_target(name: &str, capture_file: &Path) -> Arc<crate::mcp::router::McpTarget> {
		Arc::new(crate::mcp::router::McpTarget {
			name: name.into(),
			spec: crate::types::agent::McpTargetSpec::Stdio {
				cmd: "sh".into(),
				args: vec![
					"-c".into(),
					"while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$CAPTURE_FILE\"; done".into(),
				],
				env: HashMap::from([(
					"CAPTURE_FILE".to_string(),
					capture_file.display().to_string(),
				)]),
			},
			backend: None,
			always_use_prefix: false,
			backend_policies: Default::default(),
		})
	}

	async fn wait_until_contains(path: &Path, needle: &str) {
		let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
		loop {
			let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
			if content.contains(needle) {
				return;
			}
			if tokio::time::Instant::now() >= deadline {
				panic!(
					"capture {:?} did not contain {:?}; last content: {:?}",
					path, needle, content
				);
			}
			tokio::time::sleep(Duration::from_millis(20)).await;
		}
	}

	async fn assert_not_contains_for(path: &Path, needle: &str, duration: Duration) {
		let deadline = tokio::time::Instant::now() + duration;
		loop {
			let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
			if content.contains(needle) {
				panic!(
					"capture {:?} unexpectedly contained {:?}; content: {:?}",
					path, needle, content
				);
			}
			if tokio::time::Instant::now() >= deadline {
				return;
			}
			tokio::time::sleep(Duration::from_millis(20)).await;
		}
	}

	fn single_capture_relay(allow_degraded: bool, capture_file: &Path) -> Result<Relay, mcp::Error> {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		Relay::new(
			McpBackendGroup {
				targets: vec![capture_target("serverA", capture_file)],
				stateful: false,
				allow_degraded,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
	}

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

	#[tokio::test]
	async fn merge_tasks_drops_pagination_when_multiple_upstreams_participate() {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let mut a = ListTasksResult::new(vec![rmcp::model::Task::new(
			"task-1".to_string(),
			rmcp::model::TaskStatus::Working,
			"2026-01-01T00:00:00Z".to_string(),
			"2026-01-01T00:00:00Z".to_string(),
		)]);
		a.next_cursor = Some("cursor-a".to_string());
		a.total = Some(1);

		let mut b = ListTasksResult::new(vec![rmcp::model::Task::new(
			"task-2".to_string(),
			rmcp::model::TaskStatus::Working,
			"2026-01-01T00:00:00Z".to_string(),
			"2026-01-01T00:00:00Z".to_string(),
		)]);
		b.next_cursor = Some("cursor-b".to_string());
		b.total = Some(1);

		let merged = relay.merge_tasks(CelExecWrapper::new(Arc::new(None)))(vec![
			(strng::new("serverA"), a.into()),
			(strng::new("serverB"), b.into()),
		])
		.expect("merge should succeed");

		let ServerResult::ListTasksResult(result) = merged else {
			panic!("expected list/tasks result");
		};
		assert_eq!(result.tasks.len(), 2);
		assert_eq!(result.next_cursor, None);
		assert_eq!(result.total, None);
	}

	#[test]
	fn wrap_resource_uri_preserves_template_braces() {
		let wrapped = wrap_resource_uri(None, "counter", "memo://{bucket}/path");
		assert!(wrapped.contains("{bucket}"));
	}

	#[rstest]
	#[case("counter", "memo://insights")]
	#[case("server_01", "memo://{bucket}/path")]
	#[case("api-prod", "https://example.com/a path?q=a+b&x=1")]
	#[case("service9", "urn:uuid:550e8400-e29b-41d4-a716-446655440000")]
	#[case("n0de-1", "custom://emoji/\u{2603}")]
	fn wrap_resource_uri_roundtrip(#[case] target: &str, #[case] uri: &str) {
		let wrapped = wrap_resource_uri(None, target, uri).into_owned();
		let parsed = url::Url::parse(&wrapped).expect("wrapped uri should be valid");
		assert_eq!(parsed.scheme(), AGW_SCHEME);
		let decoded_target = parsed.host_str().and_then(decode_target_from_uri_host);
		assert_eq!(decoded_target.as_deref(), Some(target));
		let extracted = parsed
			.query_pairs()
			.find(|(k, _)| k == AGW_URI_QUERY_PARAM)
			.map(|(_, v)| v.into_owned())
			.expect("wrapped uri should contain u param");
		assert_eq!(extracted, uri);
	}

	#[rstest]
	#[case("serverA", "resource")]
	#[case("api1", "name_with_underscores")]
	#[case("node9", "dash-and.dot")]
	fn resource_name_prefixes_when_multiplexing(#[case] target: &str, #[case] name: &str) {
		let prefixed = resource_name(None, target, Cow::Borrowed(name));
		assert_eq!(prefixed, format!("{target}{TARGET_NAME_DELIMITER}{name}"));
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
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let id = relay
			.encode_upstream_request_id("server::with::colons", &RequestId::Number(1))
			.expect("request id should encode");
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
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let original = RequestId::String("my::id:with::separators".into());
		let id = relay
			.encode_upstream_request_id("server::with::colons", &original)
			.expect("request id should encode");
		let (decoded_name, decoded_id) = relay
			.decode_upstream_request_id(&id)
			.expect("decode failed");
		assert_eq!(decoded_name, "server::with::colons");
		assert_eq!(decoded_id, original);
	}

	#[tokio::test]
	async fn decode_upstream_request_id_rejects_mismatched_session_binding() {
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
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");
		let mismatched_relay = Relay::new(
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
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let id = relay
			.encode_upstream_request_id("serverA", &RequestId::Number(1))
			.expect("request id should encode");
		let err = mismatched_relay
			.decode_upstream_request_id(&id)
			.expect_err("mismatched session binding should be rejected");
		assert!(matches!(err, UpstreamError::InvalidRequest(_)));
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
				allow_degraded: false,
				allow_insecure_multiplex: false,
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
				allow_degraded: false,
				allow_insecure_multiplex: false,
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
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let rewritten_id = relay
			.encode_upstream_request_id("serverA", &RequestId::Number(1))
			.expect("request id should encode");
		let encoded_id = rewritten_id.to_string();
		let notification = ClientNotification::CancelledNotification(CancelledNotification::new(
			CancelledNotificationParam {
				request_id: rewritten_id,
				reason: Some("client cancelled".to_string()),
			},
		));

		let r = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification,
		};

		let ctx = IncomingRequestContext::empty();
		let result = relay.send_notification(r, ctx).await;
		assert!(result.is_ok());

		wait_until_contains(server_a_capture.path(), "\"notifications/cancelled\"").await;
		wait_until_contains(server_a_capture.path(), "\"requestId\":1").await;
		let server_a_raw = tokio::fs::read_to_string(server_a_capture.path())
			.await
			.unwrap_or_default();
		assert!(
			!server_a_raw.contains(&encoded_id),
			"serverA saw encoded request id; expected rewritten id. raw={server_a_raw:?}"
		);

		assert_not_contains_for(
			server_b_capture.path(),
			"\"notifications/cancelled\"",
			Duration::from_millis(500),
		)
		.await;
	}

	#[tokio::test]
	async fn send_notification_cancelled_mismatched_session_binding_should_be_dropped() {
		use rmcp::model::{CancelledNotification, CancelledNotificationParam, JsonRpcNotification};

		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");
		let mismatched_relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let rewritten_id = relay
			.encode_upstream_request_id("serverA", &RequestId::Number(1))
			.expect("request id should encode");
		let notification = ClientNotification::CancelledNotification(CancelledNotification::new(
			CancelledNotificationParam {
				request_id: rewritten_id,
				reason: Some("client cancelled".to_string()),
			},
		));
		let r = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification,
		};

		let ctx = IncomingRequestContext::empty();
		let result = mismatched_relay.send_notification(r, ctx).await;
		assert!(result.is_ok());

		assert_not_contains_for(
			server_a_capture.path(),
			"\"notifications/cancelled\"",
			Duration::from_millis(500),
		)
		.await;
		assert_not_contains_for(
			server_b_capture.path(),
			"\"notifications/cancelled\"",
			Duration::from_millis(500),
		)
		.await;
	}

	#[tokio::test]
	async fn send_notification_progress_should_route_to_single_upstream_and_rewrite_token() {
		use rmcp::model::{
			JsonRpcNotification, ProgressNotification, ProgressNotificationParam, ProgressToken,
		};

		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let rewritten_progress_token = relay
			.encode_upstream_progress_token("serverA", &ProgressToken(RequestId::Number(1)))
			.expect("progress token should encode");
		let encoded_token = rewritten_progress_token.0.to_string();
		let notification = ClientNotification::ProgressNotification(ProgressNotification::new(
			ProgressNotificationParam::new(rewritten_progress_token, 0.5)
				.with_total(1.0)
				.with_message("halfway"),
		));

		let r = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification,
		};

		let ctx = IncomingRequestContext::empty();
		let result = relay.send_notification(r, ctx).await;
		assert!(result.is_ok());

		wait_until_contains(server_a_capture.path(), "\"notifications/progress\"").await;
		wait_until_contains(server_a_capture.path(), "\"progressToken\":1").await;
		let server_a_raw = tokio::fs::read_to_string(server_a_capture.path())
			.await
			.unwrap_or_default();
		assert!(
			!server_a_raw.contains(&encoded_token),
			"serverA saw encoded progress token; expected rewritten token. raw={server_a_raw:?}"
		);

		assert_not_contains_for(
			server_b_capture.path(),
			"\"notifications/progress\"",
			Duration::from_millis(500),
		)
		.await;
	}

	#[tokio::test]
	async fn map_server_message_rewrites_url_elicitation_identifiers() {
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
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(7),
			request: rmcp::model::ServerRequest::CreateElicitationRequest(
				rmcp::model::CreateElicitationRequest::new(
					rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
						meta: None,
						message: "Open a URL".to_string(),
						url: "https://example.com/flow".to_string(),
						elicitation_id: "elicit-1".to_string(),
					},
				),
			),
		});
		let rewritten_request = relay
			.map_server_message("serverA", request_message)
			.expect("server request should rewrite");
		let ServerJsonRpcMessage::Request(req) = rewritten_request else {
			panic!("expected server request");
		};
		let (request_target, request_id) = relay
			.decode_upstream_request_id(&req.id)
			.expect("request id should decode");
		assert_eq!(request_target, "serverA");
		assert_eq!(request_id, RequestId::Number(7));
		let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
			panic!("expected create elicitation request");
		};
		let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
			elicitation_id, ..
		} = create_req.params
		else {
			panic!("expected URL elicitation params");
		};
		let (target, original_id) = relay
			.decode_upstream_elicitation_id(&elicitation_id)
			.expect("elicitation id should decode");
		assert_eq!(target, "serverA");
		assert_eq!(original_id, "elicit-1");

		let completion_message = ServerJsonRpcMessage::Notification(JsonRpcNotification {
			jsonrpc: Default::default(),
			notification: ServerNotification::ElicitationCompletionNotification(
				rmcp::model::ElicitationCompletionNotification::new(
					rmcp::model::ElicitationResponseNotificationParam::new("elicit-2"),
				),
			),
		});
		let rewritten_completion = relay
			.map_server_message("serverA", completion_message)
			.expect("completion notification should rewrite");
		let ServerJsonRpcMessage::Notification(notification) = rewritten_completion else {
			panic!("expected server notification");
		};
		let ServerNotification::ElicitationCompletionNotification(completion) =
			notification.notification
		else {
			panic!("expected elicitation completion notification");
		};
		let (target, original_id) = relay
			.decode_upstream_elicitation_id(&completion.params.elicitation_id)
			.expect("elicitation completion id should decode");
		assert_eq!(target, "serverA");
		assert_eq!(original_id, "elicit-2");
	}

	#[tokio::test]
	async fn send_notification_elicitation_response_should_route_to_single_upstream_and_rewrite_id() {
		use rmcp::model::JsonRpcNotification;

		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(11),
			request: rmcp::model::ServerRequest::CreateElicitationRequest(
				rmcp::model::CreateElicitationRequest::new(
					rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
						meta: None,
						message: "Open a URL".to_string(),
						url: "https://example.com/flow".to_string(),
						elicitation_id: "elicit-1".to_string(),
					},
				),
			),
		});
		let rewritten_request = relay
			.map_server_message("serverA", request_message)
			.expect("server request should rewrite");
		let ServerJsonRpcMessage::Request(req) = rewritten_request else {
			panic!("expected server request");
		};
		let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
			panic!("expected create elicitation request");
		};
		let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
			elicitation_id: encoded,
			..
		} = create_req.params
		else {
			panic!("expected URL elicitation params");
		};

		let notification =
			ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
				ELICITATION_RESPONSE_METHOD,
				Some(json!({
					"elicitationId": encoded,
					"action": "accept"
				})),
			));
		let r = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification,
		};

		let ctx = IncomingRequestContext::empty();
		let result = relay.send_notification(r, ctx).await;
		assert!(result.is_ok());

		wait_until_contains(
			server_a_capture.path(),
			"\"notifications/elicitation/response\"",
		)
		.await;
		wait_until_contains(server_a_capture.path(), "\"elicitationId\":\"elicit-1\"").await;
		let server_a_raw = tokio::fs::read_to_string(server_a_capture.path())
			.await
			.unwrap_or_default();
		assert!(
			!server_a_raw.contains("agw::"),
			"serverA saw encoded elicitation id; expected rewritten id. raw={server_a_raw:?}"
		);

		assert_not_contains_for(
			server_b_capture.path(),
			"\"notifications/elicitation/response\"",
			Duration::from_millis(500),
		)
		.await;
	}

	#[tokio::test]
	async fn send_notification_elicitation_response_untracked_id_should_be_dropped() {
		use rmcp::model::JsonRpcNotification;

		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let encoded = relay
			.encode_upstream_elicitation_id("serverA", "elicit-untracked")
			.expect("elicitation id should encode");
		let notification =
			ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
				ELICITATION_RESPONSE_METHOD,
				Some(json!({
					"elicitationId": encoded,
					"action": "accept"
				})),
			));
		let r = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification,
		};

		let ctx = IncomingRequestContext::empty();
		let result = relay.send_notification(r, ctx).await;
		assert!(result.is_ok());

		assert_not_contains_for(
			server_a_capture.path(),
			"\"notifications/elicitation/response\"",
			Duration::from_millis(500),
		)
		.await;
		assert_not_contains_for(
			server_b_capture.path(),
			"\"notifications/elicitation/response\"",
			Duration::from_millis(500),
		)
		.await;
	}

	#[tokio::test]
	async fn send_notification_elicitation_response_session_binding_mismatch_should_be_dropped() {
		use rmcp::model::JsonRpcNotification;

		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");
		let mismatched_relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let encoded = relay
			.encode_upstream_elicitation_id("serverA", "elicit-mismatch")
			.expect("elicitation id should encode");
		let notification =
			ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
				ELICITATION_RESPONSE_METHOD,
				Some(json!({
					"elicitationId": encoded,
					"action": "accept"
				})),
			));
		let r = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification,
		};

		let ctx = IncomingRequestContext::empty();
		let result = mismatched_relay.send_notification(r, ctx).await;
		assert!(result.is_ok());

		assert_not_contains_for(
			server_a_capture.path(),
			"\"notifications/elicitation/response\"",
			Duration::from_millis(500),
		)
		.await;
		assert_not_contains_for(
			server_b_capture.path(),
			"\"notifications/elicitation/response\"",
			Duration::from_millis(500),
		)
		.await;
	}

	#[tokio::test]
	async fn send_notification_elicitation_response_duplicate_should_be_dropped() {
		use rmcp::model::JsonRpcNotification;

		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(17),
			request: rmcp::model::ServerRequest::CreateElicitationRequest(
				rmcp::model::CreateElicitationRequest::new(
					rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
						meta: None,
						message: "Open a URL".to_string(),
						url: "https://example.com/flow".to_string(),
						elicitation_id: "elicit-dup".to_string(),
					},
				),
			),
		});
		let rewritten_request = relay
			.map_server_message("serverA", request_message)
			.expect("server request should rewrite");
		let ServerJsonRpcMessage::Request(req) = rewritten_request else {
			panic!("expected server request");
		};
		let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
			panic!("expected create elicitation request");
		};
		let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
			elicitation_id: encoded,
			..
		} = create_req.params
		else {
			panic!("expected URL elicitation params");
		};

		let send_response = |elicitation_id: &str| JsonRpcNotification {
			jsonrpc: Default::default(),
			notification: ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
				ELICITATION_RESPONSE_METHOD,
				Some(json!({
					"elicitationId": elicitation_id,
					"action": "accept"
				})),
			)),
		};

		let ctx = IncomingRequestContext::empty();
		relay
			.send_notification(send_response(&encoded), ctx.clone())
			.await
			.expect("first response should be accepted");
		relay
			.send_notification(send_response(&encoded), ctx)
			.await
			.expect("duplicate response should be ignored");

		wait_until_contains(
			server_a_capture.path(),
			"\"notifications/elicitation/response\"",
		)
		.await;
		let server_a_raw = tokio::fs::read_to_string(server_a_capture.path())
			.await
			.unwrap_or_default();
		assert_eq!(
			server_a_raw
				.matches("\"notifications/elicitation/response\"")
				.count(),
			1,
			"duplicate elicitation response should not be forwarded twice"
		);
		assert_not_contains_for(
			server_b_capture.path(),
			"\"notifications/elicitation/response\"",
			Duration::from_millis(500),
		)
		.await;
	}

	#[tokio::test]
	async fn send_notification_elicitation_response_post_resume_should_be_dropped() {
		use rmcp::model::JsonRpcNotification;

		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let server_b_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay")
		.with_session_binding("session-1", Encoder::base64());
		let resumed_relay = Relay::new(
			McpBackendGroup {
				targets: vec![
					capture_target("serverA", server_a_capture.path()),
					capture_target("serverB", server_b_capture.path()),
				],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay")
		.with_session_binding("session-1", Encoder::base64());

		let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(19),
			request: rmcp::model::ServerRequest::CreateElicitationRequest(
				rmcp::model::CreateElicitationRequest::new(
					rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
						meta: None,
						message: "Open a URL".to_string(),
						url: "https://example.com/flow".to_string(),
						elicitation_id: "elicit-resume".to_string(),
					},
				),
			),
		});
		let rewritten_request = relay
			.map_server_message("serverA", request_message)
			.expect("server request should rewrite");
		let ServerJsonRpcMessage::Request(req) = rewritten_request else {
			panic!("expected server request");
		};
		let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
			panic!("expected create elicitation request");
		};
		let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
			elicitation_id: encoded,
			..
		} = create_req.params
		else {
			panic!("expected URL elicitation params");
		};

		let notification =
			ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
				ELICITATION_RESPONSE_METHOD,
				Some(json!({
					"elicitationId": encoded,
					"action": "accept"
				})),
			));
		let r = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification,
		};

		let ctx = IncomingRequestContext::empty();
		let result = resumed_relay.send_notification(r, ctx).await;
		assert!(result.is_ok());

		assert_not_contains_for(
			server_a_capture.path(),
			"\"notifications/elicitation/response\"",
			Duration::from_millis(500),
		)
		.await;
		assert_not_contains_for(
			server_b_capture.path(),
			"\"notifications/elicitation/response\"",
			Duration::from_millis(500),
		)
		.await;
	}

	#[tokio::test]
	async fn send_notification_elicitation_response_single_target_tracked_id_should_route() {
		use rmcp::model::JsonRpcNotification;

		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let server_a_capture = NamedTempFile::new().expect("temp file");
		let relay = Relay::new(
			McpBackendGroup {
				targets: vec![capture_target("serverA", server_a_capture.path())],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect("relay");

		let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
			jsonrpc: Default::default(),
			id: RequestId::Number(13),
			request: rmcp::model::ServerRequest::CreateElicitationRequest(
				rmcp::model::CreateElicitationRequest::new(
					rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
						meta: None,
						message: "Open a URL".to_string(),
						url: "https://example.com/flow".to_string(),
						elicitation_id: "elicit-single-target".to_string(),
					},
				),
			),
		});
		let rewritten_request = relay
			.map_server_message("serverA", request_message)
			.expect("server request should register elicitation");
		let ServerJsonRpcMessage::Request(req) = rewritten_request else {
			panic!("expected server request");
		};
		let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
			panic!("expected create elicitation request");
		};
		let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
			elicitation_id, ..
		} = create_req.params
		else {
			panic!("expected URL elicitation params");
		};

		let notification =
			ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
				ELICITATION_RESPONSE_METHOD,
				Some(json!({
					"elicitationId": elicitation_id,
					"action": "accept"
				})),
			));
		let r = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification,
		};

		let ctx = IncomingRequestContext::empty();
		let result = relay.send_notification(r, ctx).await;
		assert!(result.is_ok());

		wait_until_contains(
			server_a_capture.path(),
			"\"notifications/elicitation/response\"",
		)
		.await;
		wait_until_contains(
			server_a_capture.path(),
			"\"elicitationId\":\"elicit-single-target\"",
		)
		.await;
	}

	#[tokio::test]
	async fn send_fanout_deletion_allow_degraded_true_ignores_cleanup_errors() {
		let capture = NamedTempFile::new().expect("temp file");
		let relay = single_capture_relay(true, capture.path()).expect("relay with allow_degraded=true");
		let ctx = IncomingRequestContext::empty();

		let first = relay.send_fanout_deletion(ctx.clone()).await;
		assert!(first.is_ok(), "first deletion should succeed");

		let second = relay.send_fanout_deletion(ctx).await;
		assert!(
			second.is_ok(),
			"degraded deletion should ignore cleanup errors"
		);
	}

	#[tokio::test]
	async fn send_fanout_deletion_allow_degraded_false_returns_cleanup_error() {
		let capture = NamedTempFile::new().expect("temp file");
		let relay =
			single_capture_relay(false, capture.path()).expect("relay with allow_degraded=false");
		let ctx = IncomingRequestContext::empty();

		let first = relay.send_fanout_deletion(ctx.clone()).await;
		assert!(first.is_ok(), "first deletion should succeed");

		let second = relay.send_fanout_deletion(ctx).await;
		assert!(
			second.is_err(),
			"strict deletion should return cleanup errors"
		);
	}

	#[tokio::test]
	async fn send_notification_allow_degraded_true_ignores_delivery_errors() {
		let capture = NamedTempFile::new().expect("temp file");
		let relay = single_capture_relay(true, capture.path()).expect("relay with allow_degraded=true");
		let ctx = IncomingRequestContext::empty();

		let deleted = relay.send_fanout_deletion(ctx.clone()).await;
		assert!(deleted.is_ok(), "upstream shutdown should succeed");

		let notification = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification: ClientNotification::InitializedNotification(
				rmcp::model::InitializedNotification {
					method: Default::default(),
					extensions: Default::default(),
				},
			),
		};
		let result = relay.send_notification(notification, ctx).await;
		assert!(
			result.is_ok(),
			"degraded mode should ignore notification delivery errors"
		);
	}

	#[tokio::test]
	async fn send_notification_allow_degraded_false_returns_delivery_error() {
		let capture = NamedTempFile::new().expect("temp file");
		let relay =
			single_capture_relay(false, capture.path()).expect("relay with allow_degraded=false");
		let ctx = IncomingRequestContext::empty();

		let deleted = relay.send_fanout_deletion(ctx.clone()).await;
		assert!(deleted.is_ok(), "upstream shutdown should succeed");

		let notification = JsonRpcNotification {
			jsonrpc: Default::default(),
			notification: ClientNotification::InitializedNotification(
				rmcp::model::InitializedNotification {
					method: Default::default(),
					extensions: Default::default(),
				},
			),
		};
		let result = relay.send_notification(notification, ctx).await;
		assert!(
			result.is_err(),
			"strict mode should return notification delivery errors"
		);
	}

	#[test]
	fn relay_rejects_backend_name_containing_delimiter() {
		let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
		let err = Relay::new(
			McpBackendGroup {
				targets: vec![Arc::new(crate::mcp::router::McpTarget {
					name: "bad__name".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				})],
				stateful: false,
				allow_degraded: false,
				allow_insecure_multiplex: false,
			},
			McpAuthorizationSet::new(vec![].into()),
			PolicyClient {
				inputs: test.inputs(),
			},
		)
		.expect_err("should reject backend name containing delimiter");
		assert!(
			err.to_string().contains("reserved delimiter"),
			"unexpected error: {err}"
		);
	}
}
