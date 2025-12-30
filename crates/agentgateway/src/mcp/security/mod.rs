// MCP Security Guards Framework
//
// This module provides a pluggable security guard system for MCP protocol operations.
// Guards can inspect and modify requests/responses to detect and prevent security threats
// specific to the Model Context Protocol.
//
// Architecture:
// - Native guards: Compiled into binary, fastest performance (< 1ms latency)
// - WASM guards: Loaded at runtime, good performance (~5-10ms latency)
// - External guards: Webhook/gRPC services for complex analysis

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub mod native;
pub mod wasm;

// Re-export core types
pub use native::{ToolPoisoningDetector, RugPullDetector, ToolShadowingDetector, ServerWhitelistChecker};

/// Security guard that can be applied to MCP protocol operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSecurityGuard {
    /// Unique identifier for this guard
    pub id: String,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Execution priority (lower = runs first)
    #[serde(default = "default_priority")]
    pub priority: u32,

    /// Behavior when guard fails to execute
    #[serde(default)]
    pub failure_mode: FailureMode,

    /// Maximum time allowed for guard execution
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,

    /// Which phases this guard runs on
    #[serde(default)]
    pub runs_on: Vec<GuardPhase>,

    /// Whether guard is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// The specific guard implementation
    #[serde(flatten)]
    pub kind: McpGuardKind,
}

fn default_priority() -> u32 {
    100
}

fn default_timeout() -> u64 {
    100
}

fn default_enabled() -> bool {
    true
}

/// Guard implementation types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpGuardKind {
    /// Tool Poisoning Detection (native)
    ToolPoisoning(native::ToolPoisoningConfig),

    /// Rug Pull Detection (native)
    RugPull(native::RugPullConfig),

    /// Tool Shadowing Prevention (native)
    ToolShadowing(native::ToolShadowingConfig),

    /// Server Whitelist Enforcement (native)
    ServerWhitelist(native::ServerWhitelistConfig),

    /// Custom WASM module
    #[cfg(feature = "wasm-guards")]
    Wasm(wasm::WasmGuardConfig),
}

/// Execution phase for guards
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardPhase {
    /// Before forwarding client request to MCP server
    Request,

    /// After receiving response from MCP server
    Response,

    /// Specifically for tools/list responses
    ToolsList,

    /// Specifically for tool invocations (tools/call)
    ToolInvoke,
}

impl Default for GuardPhase {
    fn default() -> Self {
        GuardPhase::Request
    }
}

/// How to behave when guard execution fails (timeout, error, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureMode {
    /// Block request on failure (secure default)
    FailClosed,

    /// Allow request on failure (availability over security)
    FailOpen,
}

impl Default for FailureMode {
    fn default() -> Self {
        FailureMode::FailClosed
    }
}

/// Decision made by a security guard
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardDecision {
    /// Allow the operation to proceed
    Allow,

    /// Block the operation
    Deny(DenyReason),

    /// Modify the request/response
    Modify(ModifyAction),
}

/// Reason for denying an operation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DenyReason {
    /// Short reason code (e.g., "tool_poisoning_detected")
    pub code: String,

    /// Human-readable message
    pub message: String,

    /// Optional details for debugging/auditing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Action to modify request/response
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModifyAction {
    /// Mask sensitive data in response
    MaskFields(Vec<String>),

    /// Add warning headers
    AddWarning(String),

    /// Transform content
    Transform(serde_json::Value),
}

/// Context provided to guards for evaluation
#[derive(Debug, Clone)]
pub struct GuardContext {
    /// Server/target name
    pub server_name: String,

    /// Optional session/user identity
    pub identity: Option<String>,

    /// Request metadata
    pub metadata: serde_json::Value,
}

/// Result of guard execution
pub type GuardResult = Result<GuardDecision, GuardError>;

/// Errors that can occur during guard execution
#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error("Guard execution timeout after {0:?}")]
    Timeout(Duration),

    #[error("Guard execution error: {0}")]
    ExecutionError(String),

    #[error("Guard configuration error: {0}")]
    ConfigError(String),

    #[error("WASM module error: {0}")]
    #[cfg(feature = "wasm-guards")]
    WasmError(String),
}

use std::sync::Arc;

/// Guard executor that manages and executes security guards in priority order
#[derive(Clone)]
pub struct GuardExecutor {
	guards: Arc<Vec<InitializedGuard>>,
}

struct InitializedGuard {
	config: McpSecurityGuard,
	guard: Arc<dyn native::NativeGuard>,
}

impl GuardExecutor {
	/// Create a new GuardExecutor from a list of guard configurations
	pub fn new(configs: Vec<McpSecurityGuard>) -> Result<Self, GuardError> {
		let mut guards = Vec::new();

		for config in configs {
			if !config.enabled {
				continue;
			}

			let guard: Arc<dyn native::NativeGuard> = match &config.kind {
				McpGuardKind::ToolPoisoning(cfg) => {
					Arc::new(native::ToolPoisoningDetector::new(cfg.clone())?)
				},
				McpGuardKind::RugPull(cfg) => {
					Arc::new(native::RugPullDetector::new(cfg.clone()))
				},
				McpGuardKind::ToolShadowing(cfg) => {
					Arc::new(native::ToolShadowingDetector::new(cfg.clone()))
				},
				McpGuardKind::ServerWhitelist(cfg) => {
					Arc::new(native::ServerWhitelistChecker::new(cfg.clone()))
				},
				#[cfg(feature = "wasm-guards")]
				McpGuardKind::Wasm(_cfg) => {
					// WASM guards need special handling
					return Err(GuardError::ConfigError(
						"WASM guards not yet fully implemented".to_string()
					));
				},
			};

			guards.push(InitializedGuard {
				config: config.clone(),
				guard,
			});
		}

		// Sort by priority (lower = higher priority)
		guards.sort_by_key(|g| g.config.priority);

		Ok(Self {
			guards: Arc::new(guards),
		})
	}

	/// Create an empty executor with no guards
	pub fn empty() -> Self {
		Self {
			guards: Arc::new(Vec::new()),
		}
	}

	/// Execute guards on a tools/list response
	pub fn evaluate_tools_list(
		&self,
		tools: &[rmcp::model::Tool],
		context: &GuardContext,
	) -> GuardResult {
		for guard_entry in self.guards.iter() {
			// Only run guards configured for ToolsList or Response phase
			if !guard_entry.config.runs_on.contains(&GuardPhase::ToolsList)
				&& !guard_entry.config.runs_on.contains(&GuardPhase::Response)
			{
				continue;
			}

			// Execute guard with timeout
			let result = self.execute_with_timeout(
				|| guard_entry.guard.evaluate_tools_list(tools, context),
				Duration::from_millis(guard_entry.config.timeout_ms),
				&guard_entry.config,
			);

			// Handle result based on failure mode
			match result {
				Ok(GuardDecision::Allow) => continue,
				Ok(decision) => return Ok(decision),
				Err(e) => {
					match guard_entry.config.failure_mode {
						FailureMode::FailClosed => {
							return Err(GuardError::ExecutionError(format!(
								"Guard {} failed: {}",
								guard_entry.config.id,
								e
							)));
						},
						FailureMode::FailOpen => {
							tracing::warn!("Guard {} failed but continuing due to fail_open: {}",
								guard_entry.config.id, e);
							continue;
						},
					}
				},
			}
		}

		Ok(GuardDecision::Allow)
	}

	fn execute_with_timeout<F>(
		&self,
		f: F,
		_timeout: Duration,
		_config: &McpSecurityGuard,
	) -> GuardResult
	where
		F: FnOnce() -> GuardResult,
	{
		// TODO: Implement actual timeout mechanism using tokio::time::timeout
		// For now, just execute synchronously
		f()
	}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guard_deserialization() {
        let yaml = r#"
id: test-guard
priority: 100
failure_mode: fail_closed
timeout_ms: 50
runs_on:
  - response
type: tool_poisoning
strict_mode: true
custom_patterns:
  - "(?i)SYSTEM:\\s*override"
"#;

        let guard: McpSecurityGuard = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(guard.id, "test-guard");
        assert_eq!(guard.priority, 100);
        assert_eq!(guard.timeout_ms, 50);
        assert!(matches!(guard.kind, McpGuardKind::ToolPoisoning(_)));
    }
}
