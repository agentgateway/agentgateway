// Rug Pull Detection
//
// Monitors tool availability and integrity over time to detect sudden changes
// that could indicate a malicious server is "pulling the rug" by removing or
// modifying critical tools.
//
// NOTE: This is a placeholder implementation. Full implementation requires:
// - Persistent storage for tool baselines (Redis/file-based)
// - Change tracking and risk scoring
// - Alerting on suspicious modifications

use serde::{Deserialize, Serialize};

use super::NativeGuard;
use crate::mcp::security::{GuardContext, GuardDecision, GuardResult};

/// Configuration for Rug Pull Detection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RugPullConfig {
    /// Enable baseline tracking
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Risk threshold for alerting
    #[serde(default = "default_risk_threshold")]
    pub risk_threshold: u32,
}

fn default_enabled() -> bool {
    true
}

fn default_risk_threshold() -> u32 {
    5
}

/// Rug Pull Detector implementation
pub struct RugPullDetector {
    #[allow(dead_code)]
    config: RugPullConfig,
}

impl RugPullDetector {
    pub fn new(config: RugPullConfig) -> Self {
        Self { config }
    }
}

impl NativeGuard for RugPullDetector {
    fn evaluate_tools_list(
        &self,
        _tools: &[rmcp::model::Tool],
        _context: &GuardContext,
    ) -> GuardResult {
        // TODO: Implement baseline comparison and change detection
        // For now, always allow
        Ok(GuardDecision::Allow)
    }
}
