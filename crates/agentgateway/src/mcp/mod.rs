mod handler;
mod mergestream;
mod rbac;
mod router;
mod session;
mod sse;
mod streamablehttp;
mod upstream;

use std::fmt::{Display, Error, Write};
use std::sync::Arc;

use axum_core::BoxError;
use prometheus_client::encoding::{EncodeLabelValue, LabelValueEncoder};
pub use rbac::{McpAuthorization, McpAuthorizationSet, ResourceId, ResourceType};
pub use router::App;
use thiserror::Error;

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;

#[derive(Error, Debug)]
pub enum ClientError {
	#[error("http request failed with code: {}", .0.status())]
	Status(Box<crate::http::Response>),
	#[error("http request failed: {0}")]
	General(Arc<crate::http::Error>),
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
	fn encode(&self, encoder: &mut LabelValueEncoder) -> Result<(), Error> {
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
}
