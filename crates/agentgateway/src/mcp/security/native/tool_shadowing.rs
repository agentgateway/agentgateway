// Tool Shadowing Prevention
//
// Prevents malicious MCP servers from "shadowing" legitimate tools by creating
// tools with similar names or by attempting to override protocol methods.
//
// NOTE: This is a placeholder implementation.

use serde::{Deserialize, Serialize};

use super::NativeGuard;
use crate::mcp::security::{GuardContext, GuardDecision, GuardResult};

/// Configuration for Tool Shadowing Prevention
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolShadowingConfig {
    /// Block duplicate tool names across servers
    #[serde(default = "default_block_duplicates")]
    pub block_duplicates: bool,

    /// Protected MCP protocol method names
    #[serde(default = "default_protected_names")]
    pub protected_names: Vec<String>,
}

fn default_block_duplicates() -> bool {
    true
}

fn default_protected_names() -> Vec<String> {
    vec![
        "initialize".to_string(),
        "tools/list".to_string(),
        "tools/call".to_string(),
        "prompts/list".to_string(),
        "prompts/get".to_string(),
        "resources/list".to_string(),
        "resources/read".to_string(),
    ]
}

/// Tool Shadowing Detector implementation
pub struct ToolShadowingDetector {
    #[allow(dead_code)]
    config: ToolShadowingConfig,
}

impl ToolShadowingDetector {
    pub fn new(config: ToolShadowingConfig) -> Self {
        Self { config }
    }
}

impl NativeGuard for ToolShadowingDetector {
    fn evaluate_tools_list(
        &self,
        _tools: &[rmcp::model::Tool],
        _context: &GuardContext,
    ) -> GuardResult {
        // TODO: Implement duplicate detection and shadowing prevention
        // For now, always allow
        Ok(GuardDecision::Allow)
    }
}
