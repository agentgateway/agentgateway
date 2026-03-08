//! MCP relay runtime for multiplexed upstream targets.
//!
//! `Relay` is the single owner of gateway-side MCP runtime state for one
//! downstream session. Per-target protocol state and routing helpers live in
//! sibling modules under this namespace.

mod dispatch;
mod merge;
mod routing;
mod target;

#[cfg(test)]
use merge::merge_meta;

use agent_core::strng::Strng;
use http::StatusCode;
use http::request::Parts;
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
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use crate::cel::RequestSnapshot;
use crate::http::Response;
use crate::http::sessionpersistence::{
	Encoder, MCPSnapshotMember, MCPSnapshotRouting, MCPSnapshotState,
};
use crate::mcp;
use crate::mcp::mergestream::MessageMapper;
use crate::mcp::rbac::{CelExecWrapper, McpAuthorizationSet};
use crate::mcp::router::McpBackendGroup;
use crate::mcp::session::SessionContinuity;
use crate::mcp::streamablehttp::ServerSseMessage;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};
use crate::mcp::{ClientError, MCPInfo, mergestream, rbac, upstream};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::{AsyncLog, SpanWriteOnDrop, SpanWriter};
use routing::{TARGET_NAME_DELIMITER, TargetIds, TargetRouter};
use target::TargetSession;

const ELICITATION_RESPONSE_METHOD: &str = ElicitationResponseNotificationMethod::VALUE;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ElicitationResponseParams {
	#[serde(flatten)]
	typed: ElicitationResponseNotificationParam,
	#[serde(flatten)]
	extra: Map<String, Value>,
}

#[derive(Debug, Clone)]
enum RoutedCustomNotification {
	ElicitationResponse {
		original: rmcp::model::CustomNotification,
		params: ElicitationResponseParams,
	},
	Other(rmcp::model::CustomNotification),
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

/// Multiplexing runtime for one downstream MCP session.
///
/// `Relay` is the single owner of gateway-side MCP state for a session. It
/// orchestrates fanout and targeted dispatch, while delegating target-local
/// protocol bookkeeping to [`TargetSession`].
#[derive(Debug, Clone)]
pub struct Relay {
	upstreams: Arc<upstream::UpstreamGroup>,
	pub policies: McpAuthorizationSet,
	target_router: TargetRouter,
	is_multiplexing: bool,
	allow_degraded: bool,
	allow_insecure_multiplex: bool,
	target_sessions: Arc<RwLock<HashMap<Strng, TargetSession>>>,
	target_ids: TargetIds,
}

/// Inputs required to construct a [`Relay`].
///
/// This keeps backend configuration and policy inputs separate from the relay
/// runtime that is allocated once a session opens upstream connections.
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
	pub fn set_session_binding(&self, session_handle: &str) {
		self.target_ids.set_session_binding(session_handle);
	}

	fn target_session(&self, target: &str) -> Option<TargetSession> {
		let sessions = self.target_sessions.read().unwrap_or_else(|e| {
			tracing::error!("target session map lock poisoned while reading; continuing");
			e.into_inner()
		});
		sessions.get(target).cloned()
	}

	pub fn uses_routed_identifiers(&self) -> bool {
		self.target_router.uses_target_routing()
	}

	fn register_pending_elicitation(&self, routed_id: &str, target: &str, original_id: &str) {
		let Some(target_session) = self.target_session(target) else {
			tracing::warn!(%target, "skipping pending elicitation registration: target session not found");
			return;
		};
		target_session.register_pending_elicitation(routed_id, original_id);
	}

	fn clear_pending_elicitation(&self, target: &str, original_id: &str) {
		let Some(target_session) = self.target_session(target) else {
			tracing::warn!(%target, "skipping pending elicitation clear: target session not found");
			return;
		};
		target_session.clear_pending_elicitation(original_id);
	}

	fn take_active_elicitation(&self, routed_id: &str, target: &str, original_id: &str) -> bool {
		let Some(target_session) = self.target_session(target) else {
			tracing::warn!(%target, "dropping pending elicitation consume: target session not found");
			return false;
		};
		target_session.take_active_elicitation(routed_id, original_id)
	}

	fn encode_upstream_request_id(
		&self,
		server_name: &str,
		id: &RequestId,
	) -> Result<RequestId, ClientError> {
		if !self.uses_routed_identifiers() {
			return Ok(id.clone());
		}
		self.target_ids.encode_request_id(server_name, id)
	}

	fn encode_upstream_progress_token(
		&self,
		server_name: &str,
		token: &rmcp::model::ProgressToken,
	) -> Result<rmcp::model::ProgressToken, ClientError> {
		if !self.uses_routed_identifiers() {
			return Ok(token.clone());
		}
		self.target_ids.encode_progress_token(server_name, token)
	}

	fn encode_upstream_elicitation_id(
		&self,
		server_name: &str,
		elicitation_id: &str,
	) -> Result<String, ClientError> {
		if !self.uses_routed_identifiers() {
			return Ok(elicitation_id.to_string());
		}
		self
			.target_ids
			.encode_elicitation_id(server_name, elicitation_id)
	}

	fn decode_upstream_progress_token(
		&self,
		token: &rmcp::model::ProgressToken,
	) -> Result<(String, rmcp::model::ProgressToken), UpstreamError> {
		if let Some(default) = self.target_router.default_target_name() {
			return Ok((default.to_string(), token.clone()));
		}
		self.target_ids.decode_progress_token(token)
	}

	fn decode_upstream_elicitation_id(
		&self,
		elicitation_id: &str,
	) -> Result<(String, String), UpstreamError> {
		if let Some(default) = self.target_router.default_target_name() {
			return Ok((default.to_string(), elicitation_id.to_string()));
		}
		self.target_ids.decode_elicitation_id(elicitation_id)
	}

	pub fn decode_upstream_request_id(
		&self,
		id: &RequestId,
	) -> Result<(String, RequestId), UpstreamError> {
		if let Some(default) = self.target_router.default_target_name() {
			return Ok((default.to_string(), id.clone()));
		}
		self.target_ids.decode_request_id(id)
	}

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
					*uri = self
						.target_router
						.wrap_resource_uri(server_name, uri)
						.into_owned();
				}
			},
			ServerResult::CreateTaskResult(r) => {
				self
					.target_router
					.prefix_task_id(server_name, &mut r.task.task_id);
			},
			ServerResult::ListTasksResult(r) => {
				for task in &mut r.tasks {
					self
						.target_router
						.prefix_task_id(server_name, &mut task.task_id);
				}
			},
			ServerResult::GetTaskResult(r) => {
				self
					.target_router
					.prefix_task_id(server_name, &mut r.task.task_id);
			},
			ServerResult::CancelTaskResult(r) => {
				self
					.target_router
					.prefix_task_id(server_name, &mut r.task.task_id);
			},
			_ => {},
		}
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
					tracing::debug!(%elicitation_id, "received url elicitation request");
				}
			},
			ServerJsonRpcMessage::Response(resp) => {
				self.map_server_result(server_name, &mut resp.result);
			},
			ServerJsonRpcMessage::Notification(notif) => match &mut notif.notification {
				ServerNotification::ResourceUpdatedNotification(run) => {
					run.params.uri = self
						.target_router
						.wrap_resource_uri(server_name, &run.params.uri)
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
		let upstreams = Arc::new(upstream::UpstreamGroup::new(client, backend)?);
		let target_sessions = upstreams
			.iter_named()
			.map(|(name, _)| (name, TargetSession::new()))
			.collect::<HashMap<_, _>>();
		let target_sessions = Arc::new(RwLock::new(target_sessions));
		let target_router = TargetRouter::new(routing.default_target_name);
		let target_ids = TargetIds::new();
		Ok(Self {
			upstreams,
			policies,
			target_router,
			is_multiplexing: routing.is_multiplexing,
			allow_degraded: runtime_allow_degraded,
			allow_insecure_multiplex,
			target_sessions,
			target_ids,
		})
	}

	pub fn with_policies(&self, policies: McpAuthorizationSet) -> Self {
		Self {
			upstreams: self.upstreams.clone(),
			policies,
			target_router: self.target_router.clone(),
			is_multiplexing: self.is_multiplexing,
			allow_degraded: self.allow_degraded,
			allow_insecure_multiplex: self.allow_insecure_multiplex,
			target_sessions: self.target_sessions.clone(),
			target_ids: self.target_ids.clone(),
		}
	}

	pub fn with_session_binding(mut self, session_handle: &str, route_id_encoder: Encoder) -> Self {
		self.target_ids = self
			.target_ids
			.with_session_binding(session_handle, route_id_encoder);
		self
	}

	pub fn parse_resource_name<'a, 'b: 'a>(
		&'a self,
		res: &'b str,
	) -> Result<(&'a str, &'b str), UpstreamError> {
		self.target_router.parse_resource_name(res)
	}

	pub fn unwrap_resource_uri(&self, uri: &str) -> Option<(String, String)> {
		self.target_router.unwrap_resource_uri(uri)
	}

	/// Returns a list of upstream server names that support a specific capability.
	///
	/// Used for efficient fanout (e.g., only send `list_tools` to servers that support tools).
	///
	/// # Fallback Logic
	/// If a server has not yet completed initialization (and thus is missing from `target_sessions`),
	/// we **include it in the list**. This "fail-open" behavior ensures that requests are not silently
	/// dropped during the startup race window. If the server genuinely doesn't support the feature,
	/// it will return a standard error which the merge logic handles.
	fn upstreams_with_capability(&self, check: impl Fn(&ServerCapabilities) -> bool) -> Vec<Strng> {
		self
			.upstreams
			.iter_named()
			.filter_map(|(name, _)| match self.target_session(name.as_str()) {
				Some(target_session) => target_session
					.supports_capability(|caps| check(caps))
					.unwrap_or(true)
					.then_some(name),
				// If we haven't received the initialize result yet, we assume the server *might* support it.
				// This ensures that if a client calls list_tools before initialize finishes, we don't silently drop it.
				None => Some(name),
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

	pub fn snapshot_state(&self) -> Result<MCPSnapshotState, mcp::Error> {
		let mut members = Vec::with_capacity(self.upstreams.size());
		for (name, us) in self.upstreams.iter_named() {
			let Some(info) = self
				.target_session(name.as_str())
				.and_then(|target_session| target_session.info())
			else {
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
				default_target_name: self.target_router.default_target_name_owned(),
				is_multiplexing: self.is_multiplexing,
			},
		))
	}

	pub fn restore_snapshot_state(&self, members: &[MCPSnapshotMember]) -> Result<(), mcp::Error> {
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
			let Some(target_session) = self.target_session(member.target.as_str()) else {
				return Err(mcp::Error::SendError(
					None,
					format!(
						"missing target session for restored snapshot target {}",
						member.target
					),
				));
			};
			target_session.set_info(member.info.clone());
		}
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
}

/// Initializes request-scoped logging and CEL context for MCP handling.
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

#[cfg(test)]
mod tests;
