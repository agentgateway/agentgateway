pub(crate) mod auth;
mod handler;
mod mergestream;
mod rbac;
mod router;
mod session;
mod sse;
mod streamablehttp;
mod upstream;

use std::fmt::{Display, Write};
use std::io;
use std::sync::Arc;

#[cfg(feature = "schema")]
use crate::JsonSchema;
use crate::http::SendDirectResponse;
use crate::proxy::ProxyError;
use axum_core::BoxError;
use prometheus_client::encoding::{EncodeLabelValue, LabelValueEncoder};
pub use rbac::{McpAuthorization, McpAuthorizationSet, ResourceId, ResourceType};
use rmcp::model::RequestId;
pub use router::App;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase")]
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

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;

#[derive(Error, Debug)]
pub enum Error {
	#[error("method not allowed; must be GET, POST, or DELETE")]
	MethodNotAllowed,
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
	#[error("failed to start stdio server: {0}")]
	Stdio(io::Error),
	#[error("upstream error: {}", .0.status())]
	UpstreamError(Box<SendDirectResponse>),
	#[error("send error: {}", .1)]
	SendError(Option<RequestId>, String),
	// Intentionally do NOT say its not authorized; we hide the existence of the tool
	#[error("Unknown {1}: {2}")]
	Authorization(RequestId, String, String),
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

#[derive(Default, Serialize, Deserialize, Clone, Debug, PartialEq, ::cel::DynamicType)]
#[serde(rename_all = "camelCase")]
#[dynamic(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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

#[derive(Default, Serialize, Deserialize, Clone, Debug, PartialEq, ::cel::DynamicType)]
#[serde(rename_all = "camelCase")]
#[dynamic(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
