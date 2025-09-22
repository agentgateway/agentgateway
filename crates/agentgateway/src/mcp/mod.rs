mod handler;
mod mergestream;
mod metrics;
mod rbac;
mod router;
mod session;
mod sse;
mod streamablehttp;
mod upstream;

use std::sync::Arc;

use axum_core::BoxError;
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

#[derive(Debug, Default, Clone)]
pub struct MCPInfo {
	pub tool_call_name: Option<String>,
	pub target_name: Option<String>,
}
