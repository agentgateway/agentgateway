mod apps;
pub(crate) mod auth;
pub(crate) mod guardrails;
mod handler;
mod mergestream;
mod rbac;
mod router;
mod session;
mod sse;
mod streamablehttp;
mod subscriptions;
mod upstream;

use std::fmt::{Display, Write};
use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum_core::BoxError;
use prometheus_client::encoding::{EncodeLabelValue, LabelValueEncoder};
pub use rbac::{McpAuthorization, McpAuthorizationSet, ResourceId, ResourceType};
use rmcp::model::{
	CallToolRequestMethod, CancelTaskMethod, CompleteRequestMethod, ConstString,
	DiscoverRequestMethod, ErrorCode, ErrorData, GetPromptRequestMethod, GetTaskMethod,
	GetTaskPayloadMethod, InitializeResultMethod, JsonRpcError, ListPromptsRequestMethod,
	ListResourceTemplatesRequestMethod, ListResourcesRequestMethod, ListTasksMethod,
	ListToolsRequestMethod, PingRequestMethod, ProtocolVersion, ReadResourceRequestMethod, RequestId,
	SetLevelRequestMethod, SubscribeRequestMethod, SubscriptionsListenRequestMethod,
	UnsubscribeRequestMethod,
};
pub use router::App;
use thiserror::Error;

use crate::http::SendDirectResponse;
use crate::proxy::ProxyError;
use crate::{apply, schema};

#[apply(schema!)]
#[derive(Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "schema", schemars(rename = "McpBackendFailureMode"))]
pub enum FailureMode {
	/// Fail the entire session if any target fails to initialize or any
	/// upstream fails during a fanout. This is the default and matches
	/// current behavior.
	#[default]
	FailClosed,
	/// Skip failed targets/upstreams and continue serving from healthy ones.
	/// If ALL targets fail, still return an error.
	FailOpen,
}

pub(crate) const DEFAULT_SESSION_IDLE_TTL: Duration = Duration::from_mins(30);

/// Method names of rmcp's typed `ClientRequest` variants. Keep this list in sync with rmcp rev
/// bumps; only `CustomRequest` and failed typed parses consult it, so drift cannot 404 typed
/// requests.
pub(crate) fn is_known_client_request_method(method: &str) -> bool {
	matches!(
		method,
		DiscoverRequestMethod::VALUE
			| PingRequestMethod::VALUE
			| InitializeResultMethod::VALUE
			| CompleteRequestMethod::VALUE
			| SetLevelRequestMethod::VALUE
			| GetPromptRequestMethod::VALUE
			| ListPromptsRequestMethod::VALUE
			| ListResourcesRequestMethod::VALUE
			| ListResourceTemplatesRequestMethod::VALUE
			| ReadResourceRequestMethod::VALUE
			| SubscriptionsListenRequestMethod::VALUE
			| SubscribeRequestMethod::VALUE
			| UnsubscribeRequestMethod::VALUE
			| CallToolRequestMethod::VALUE
			| ListToolsRequestMethod::VALUE
			| GetTaskMethod::VALUE
			| ListTasksMethod::VALUE
			| GetTaskPayloadMethod::VALUE
			| CancelTaskMethod::VALUE
	)
}

/// True for protocol versions in the modern (2026-07-28+) era, which negotiate via
/// `server/discover` plus per-request `_meta` rather than a session-establishing `initialize`.
pub(crate) fn is_modern_version(version: &ProtocolVersion) -> bool {
	version.as_str() >= ProtocolVersion::STANDARD_HEADERS.as_str()
}

/// Methods removed for the modern (2026-07-28+) protocol by SEP-2575/SEP-2567:
/// modern clients use `server/discover` plus per-request `_meta` instead of a
/// session-establishing `initialize`, and have no session to subscribe/set-level on.
/// Keep consistent with [`is_known_client_request_method`].
pub(crate) const REMOVED_METHODS_2026_07_28: &[&str] = &[
	InitializeResultMethod::VALUE,
	PingRequestMethod::VALUE,
	SetLevelRequestMethod::VALUE,
	SubscribeRequestMethod::VALUE,
	UnsubscribeRequestMethod::VALUE,
];

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;

#[derive(Error, Debug)]
pub enum Error {
	#[error("method not allowed; must be GET, POST, or DELETE")]
	MethodNotAllowed,
	#[error("GET event stream is not supported by any upstream")]
	GetStreamNotSupported,
	#[error("client must accept both application/json and text/event-stream")]
	InvalidAccept,
	#[error("client must accept text/event-stream")]
	InvalidAcceptGet,
	#[error("client must send application/json")]
	InvalidContentType,
	#[error("fail to deserialize request body: {0}")]
	Deserialize(crate::http::Error),
	#[error("fail to create session: {0}")]
	StartSession(crate::http::Error),
	#[error("session not found")]
	UnknownSession,
	#[error("session header is required for non-initialize requests")]
	MissingSessionHeader,
	#[error("session ID is required")]
	SessionIdRequired,
	#[error("invalid session ID header")]
	InvalidSessionIdHeader,
	#[error("invalid MCP protocol version header")]
	InvalidProtocolVersion,
	#[error("unsupported MCP protocol version: {version}")]
	UnsupportedVersion {
		request_id: Option<RequestId>,
		version: String,
		include_supported_versions: bool,
	},
	#[error("MCP protocol version header/body mismatch")]
	VersionMismatch(Option<RequestId>),
	#[error("{1} header/body mismatch")]
	HeaderBodyMismatch(Option<RequestId>, &'static str),
	#[error("invalid MCP routing header: {1}")]
	InvalidRoutingHeader(Option<RequestId>, &'static str),
	#[error("method not found: {1}")]
	MethodNotFound(Option<RequestId>, String),
	#[error("invalid request parameters: {1}")]
	InvalidParams(Option<RequestId>, String),
	#[error("failed to start stdio server: {0}")]
	Stdio(io::Error),
	#[error("upstream error: {}", .0.status())]
	UpstreamError(Box<SendDirectResponse>),
	#[error("failed to send message: {1}")]
	SendError(Option<RequestId>, String),
	/// Server-side availability/capability condition (no upstreams reachable, method unsupported by
	/// the selected transport). Maps to a JSON-RPC internal error, not invalid-params: the client's
	/// request was well-formed.
	#[error("{1}")]
	Unavailable(Option<RequestId>, String),
	// Intentionally do NOT say its not authorized; we hide the existence of the tool
	#[error("Unknown {1}: {2}")]
	Authorization(RequestId, String, String),
	#[error("mcpGuardrails rejected: {}", .1.message)]
	McpGuardrails(RequestId, rmcp::ErrorData),
	#[error("failed to process session_id query parameter")]
	InvalidSessionIdQuery,
	#[error("failed to establish get stream: {0}")]
	EstablishGetStream(String),
	#[error("failed to forward message to legacy SSE: {0}")]
	ForwardLegacySse(String),
	#[error("failed to create SSE url: {0}")]
	CreateSseUrl(String),
	#[error("failed to parse openapi: {0}")]
	OpenAPI(upstream::OpenAPIParseError),
	#[error("no backends configured")]
	NoBackends,
}

impl Error {
	pub fn jsonrpc_error_body(&self) -> Option<String> {
		let (id, error) = match self {
			Error::McpGuardrails(id, rejection) => (id.clone(), rejection.clone()),
			Error::UnsupportedVersion {
				request_id: Some(id),
				version,
				include_supported_versions,
			} => (
				id.clone(),
				ErrorData {
					code: ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
					message: self.to_string().into(),
					// This gate runs before backend selection, so it reports the gateway set.
					// With single-server discover passthrough, SEP-2575's supported/discover
					// correlation holds only when the upstream advertises a superset of this list.
					data: include_supported_versions.then(|| {
						serde_json::json!({
							"supported": ProtocolVersion::KNOWN_VERSIONS,
							"requested": version,
						})
					}),
				},
			),
			_ => {
				let (id, code) = match self {
					Error::SendError(Some(id), _) | Error::Unavailable(Some(id), _) => {
						(id.clone(), ErrorCode::INTERNAL_ERROR)
					},
					Error::Authorization(id, _, _) => (id.clone(), ErrorCode::INVALID_PARAMS),
					Error::VersionMismatch(Some(id))
					| Error::HeaderBodyMismatch(Some(id), _)
					| Error::InvalidRoutingHeader(Some(id), _) => (id.clone(), ErrorCode::HEADER_MISMATCH),
					Error::MethodNotFound(Some(id), _) => (id.clone(), ErrorCode::METHOD_NOT_FOUND),
					Error::InvalidParams(Some(id), _) => (id.clone(), ErrorCode::INVALID_PARAMS),
					_ => return None,
				};
				(
					id,
					ErrorData {
						code,
						message: self.to_string().into(),
						data: None,
					},
				)
			},
		};

		serde_json::to_string(&JsonRpcError {
			jsonrpc: Default::default(),
			id: Some(id),
			error,
		})
		.ok()
	}
}

impl From<Error> for ProxyError {
	fn from(value: Error) -> Self {
		ProxyError::MCP(value)
	}
}
impl<T> From<Error> for Result<T, ProxyError> {
	fn from(val: Error) -> Self {
		Err(ProxyError::MCP(val))
	}
}

#[derive(Error, Debug)]
pub enum ClientError {
	#[error("http request failed with code: {}", .0.status())]
	Status(Box<crate::http::Response>),
	#[error("http request failed: {0}")]
	General(Arc<crate::http::Error>),
	#[error("http request failed: {0}")]
	Proxy(#[from] ProxyError),
}

impl ClientError {
	pub fn new(error: impl Into<BoxError>) -> Self {
		Self::General(Arc::new(crate::http::Error::new(error.into())))
	}
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum MCPOperation {
	Tool,
	Prompt,
	Resource,
	ResourceTemplates,
}

impl EncodeLabelValue for MCPOperation {
	fn encode(&self, encoder: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
		encoder.write_str(&self.to_string())
	}
}

impl Display for MCPOperation {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			MCPOperation::Tool => write!(f, "tool"),
			MCPOperation::Prompt => write!(f, "prompt"),
			MCPOperation::Resource => write!(f, "resource"),
			MCPOperation::ResourceTemplates => write!(f, "templates"),
		}
	}
}

#[apply(schema!)]
#[derive(Default, PartialEq, ::cel::DynamicType)]
#[dynamic(rename_all = "camelCase")]
pub struct MCPTool {
	/// The target handling the tool call after multiplexing resolution.
	pub target: String,
	/// The resolved tool name sent to the upstream target.
	pub name: String,
	/// The JSON arguments passed to the tool call.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub arguments: Option<serde_json::Map<String, serde_json::Value>>,
	/// The terminal tool result payload, if available.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub result: Option<serde_json::Value>,
	/// The terminal JSON-RPC error payload, if available.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<serde_json::Value>,
}

#[apply(schema!)]
#[derive(Default, PartialEq, ::cel::DynamicType)]
#[dynamic(rename_all = "camelCase")]
pub struct MCPInfo {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub method_name: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub session_id: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tool: Option<MCPTool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prompt: Option<ResourceId>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub resource: Option<ResourceId>,
}

impl MCPInfo {
	pub fn is_empty(&self) -> bool {
		self.method_name.is_none()
			&& self.session_id.is_none()
			&& self.tool.is_none()
			&& self.prompt.is_none()
			&& self.resource.is_none()
	}

	pub fn resource_type(&self) -> Option<MCPOperation> {
		if self.tool.is_some() {
			Some(MCPOperation::Tool)
		} else if self.prompt.is_some() {
			Some(MCPOperation::Prompt)
		} else if self.resource.is_some() {
			Some(MCPOperation::Resource)
		} else {
			None
		}
	}

	pub fn target_name(&self) -> Option<&str> {
		self
			.tool
			.as_ref()
			.map(|tool| tool.target.as_str())
			.or_else(|| self.prompt.as_ref().map(ResourceId::target))
			.or_else(|| self.resource.as_ref().map(ResourceId::target))
	}

	pub fn resource_name(&self) -> Option<&str> {
		self
			.tool
			.as_ref()
			.map(|tool| tool.name.as_str())
			.or_else(|| self.prompt.as_ref().map(ResourceId::name))
			.or_else(|| self.resource.as_ref().map(ResourceId::name))
	}

	pub fn set_tool(&mut self, target: String, name: String) {
		self.prompt = None;
		self.resource = None;
		match self.tool.as_mut() {
			Some(tool) => {
				tool.target = target;
				tool.name = name;
			},
			None => {
				self.tool = Some(MCPTool {
					target,
					name,
					..Default::default()
				});
			},
		}
	}

	pub fn set_prompt(&mut self, target: String, name: String) {
		self.tool = None;
		self.resource = None;
		self.prompt = Some(ResourceId::new(target, name));
	}

	pub fn set_resource(&mut self, target: String, name: String) {
		self.tool = None;
		self.prompt = None;
		self.resource = Some(ResourceId::new(target, name));
	}

	pub fn capture_call_arguments(
		&mut self,
		arguments: Option<serde_json::Map<String, serde_json::Value>>,
	) {
		let Some(tool) = self.tool.as_mut() else {
			return;
		};

		tool.arguments = arguments;
	}

	pub fn capture_call_result<T: serde::Serialize>(&mut self, result: &T) {
		if let Some(tool) = self.tool.as_mut() {
			tool.result = serde_json::to_value(result).ok();
		}
	}

	pub fn capture_call_error<T: serde::Serialize>(&mut self, error: &T) {
		if let Some(tool) = self.tool.as_mut() {
			tool.error = serde_json::to_value(error).ok();
		}
	}
}

impl From<&ResourceType> for MCPInfo {
	fn from(value: &ResourceType) -> Self {
		match value {
			ResourceType::Tool(tool) => Self {
				tool: Some(MCPTool {
					target: tool.target().to_string(),
					name: tool.name().to_string(),
					..Default::default()
				}),
				..Default::default()
			},
			ResourceType::Prompt(prompt) => Self {
				prompt: Some(prompt.clone()),
				..Default::default()
			},
			ResourceType::Resource(resource) => Self {
				resource: Some(resource.clone()),
				..Default::default()
			},
		}
	}
}
