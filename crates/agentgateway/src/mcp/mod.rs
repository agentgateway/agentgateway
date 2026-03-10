mod auth;
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

use crate::http::SendDirectResponse;
use crate::proxy::ProxyError;
use axum_core::BoxError;
use prometheus_client::encoding::{EncodeLabelValue, LabelValueEncoder};
pub use rbac::{McpAuthorization, McpAuthorizationSet, ResourceId, ResourceType};
use rmcp::model::RequestId;
pub use router::App;
use thiserror::Error;

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;

#[derive(Error, Debug)]
pub enum Error {
	#[error("method not allowed; must be GET, POST, or DELETE")]
	MethodNotAllowed,
	#[error("client must accept both application/json and text/event-stream")]
	InvalidAccept,
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

#[derive(Debug, Default, Clone)]
pub struct MCPInfo {
	pub method_name: Option<String>,
	/// Tool name, etc
	pub resource_name: Option<String>,
	pub target_name: Option<String>,
	pub resource: Option<MCPOperation>,
	pub session_id: Option<String>,
}
