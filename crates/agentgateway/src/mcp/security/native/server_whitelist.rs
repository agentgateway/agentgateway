// Server Whitelist Enforcement
//
// Ensures that only trusted MCP servers can be accessed through the gateway.
// Detects typosquatting attempts and validates server identity.
//
// NOTE: This is a placeholder implementation.

use serde::{Deserialize, Serialize};

use super::NativeGuard;
use crate::mcp::security::{GuardContext, GuardDecision, GuardResult};

/// Configuration for Server Whitelist
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerWhitelistConfig {
    /// List of allowed server names/IDs
    #[serde(default)]
    pub allowed_servers: Vec<String>,

    /// Detect typosquatting attempts
    #[serde(default = "default_detect_typosquats")]
    pub detect_typosquats: bool,

    /// Similarity threshold for typo detection (0.0-1.0)
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f32,
}

fn default_detect_typosquats() -> bool {
    true
}

fn default_similarity_threshold() -> f32 {
    0.85
}

/// Server Whitelist Checker implementation
pub struct ServerWhitelistChecker {
    #[allow(dead_code)]
    config: ServerWhitelistConfig,
}

impl ServerWhitelistChecker {
    pub fn new(config: ServerWhitelistConfig) -> Self {
        Self { config }
    }
}

impl NativeGuard for ServerWhitelistChecker {
    fn evaluate_tools_list(
        &self,
        _tools: &[rmcp::model::Tool],
        _context: &GuardContext,
    ) -> GuardResult {
        // TODO: Implement whitelist checking and typosquatting detection
        // For now, always allow
        Ok(GuardDecision::Allow)
    }
}
