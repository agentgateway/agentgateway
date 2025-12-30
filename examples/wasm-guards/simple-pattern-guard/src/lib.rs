// Simple Pattern Guard - WASM Example
//
// This guard blocks tools that match configurable patterns.
// It demonstrates the basic structure of a WASM security guard.

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "security-guard",
    path: "../wit",
});

use exports::mcp::security_guard::guard::{
    Decision, DenyReason, Guest, GuardContext, Tool,
};

struct SimplePatternGuard;

impl Guest for SimplePatternGuard {
    fn evaluate_tools_list(
        tools: Vec<Tool>,
        context: GuardContext,
    ) -> Result<Decision, String> {
        // Get blocked patterns from config (or use defaults)
        let blocked_patterns = get_blocked_patterns();

        // Log guard execution
        log_info(&format!(
            "Evaluating {} tools from server '{}'",
            tools.len(),
            context.server_name
        ));

        // Check each tool against blocked patterns
        for tool in tools.iter() {
            // Check tool name
            for pattern in &blocked_patterns {
                if tool.name.to_lowercase().contains(&pattern.to_lowercase()) {
                    log_warn(&format!(
                        "Blocked tool '{}' matching pattern '{}'",
                        tool.name, pattern
                    ));

                    return Ok(Decision::Deny(DenyReason {
                        code: "pattern_blocked".to_string(),
                        message: format!(
                            "Tool '{}' matches blocked pattern '{}'",
                            tool.name, pattern
                        ),
                        details: Some(serde_json::json!({
                            "tool_name": tool.name,
                            "matched_pattern": pattern,
                            "server": context.server_name,
                        }).to_string()),
                    }));
                }
            }

            // Check tool description
            if let Some(desc) = &tool.description {
                for pattern in &blocked_patterns {
                    if desc.to_lowercase().contains(&pattern.to_lowercase()) {
                        log_warn(&format!(
                            "Blocked tool '{}' with description matching pattern '{}'",
                            tool.name, pattern
                        ));

                        return Ok(Decision::Deny(DenyReason {
                            code: "description_pattern_blocked".to_string(),
                            message: format!(
                                "Tool '{}' description matches blocked pattern '{}'",
                                tool.name, pattern
                            ),
                            details: Some(serde_json::json!({
                                "tool_name": tool.name,
                                "matched_pattern": pattern,
                                "description": desc,
                            }).to_string()),
                        }));
                    }
                }
            }
        }

        log_info("All tools passed pattern check");
        Ok(Decision::Allow)
    }
}

// Helper: Get blocked patterns from config or use defaults
fn get_blocked_patterns() -> Vec<String> {
    // Try to get custom patterns from config
    let config_patterns = mcp::security_guard::host::get_config("blocked_patterns");

    if !config_patterns.is_empty() {
        // Parse JSON array
        if let Ok(patterns) = serde_json::from_str::<Vec<String>>(&config_patterns) {
            return patterns;
        }
    }

    // Default patterns
    vec![
        "delete".to_string(),
        "rm -rf".to_string(),
        "drop table".to_string(),
        "eval".to_string(),
        "exec".to_string(),
    ]
}

// Logging helpers using host functions
fn log_info(msg: &str) {
    mcp::security_guard::host::log(2, msg);  // 2 = info
}

fn log_warn(msg: &str) {
    mcp::security_guard::host::log(3, msg);  // 3 = warn
}

#[allow(dead_code)]
fn log_error(msg: &str) {
    mcp::security_guard::host::log(4, msg);  // 4 = error
}

// Export the implementation
export!(SimplePatternGuard);
