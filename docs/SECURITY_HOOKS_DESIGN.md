# Security Hooks Framework - Design Document

## Executive Summary

This document defines a generic, extensible security hook system for the Agent Gateway to implement the 12 security capabilities outlined in the UNITONE Gateway Security Capabilities v1.0 document.

## Architecture Overview

### Three-Tier Implementation Strategy

```
┌─────────────────────────────────────────────────────────────┐
│                    Tier 1: Native Hooks                       │
│   (High-Performance Rust - MCP Protocol-Specific Threats)     │
│   - Tool Poisoning Detection                                  │
│   - Rug Pull Detection                                        │
│   - Tool Shadowing Prevention                                 │
└───────────────────────────────────┬─────────────────────────┘
                                    │
┌───────────────────────────────────▼─────────────────────────┐
│                Tier 2: Hybrid Hooks                           │
│   (Rust + External Services - Policy-Driven)                 │
│   - Server Spoofing & Whitelisting                           │
│   - Tool-Level Access Control (RBAC/ABAC)                    │
│   - Content Filtering & Protocol Validation                  │
│   - Token & Session Security                                 │
└───────────────────────────────────┬─────────────────────────┘
                                    │
┌───────────────────────────────────▼─────────────────────────┐
│             Tier 3: External Services                         │
│   (Webhook/gRPC - Operational & Analytics)                   │
│   - Context Integrity Validation                             │
│   - Audit Logging with MCP Correlation                       │
│   - Sensitive Data Protection (DLP)                          │
│   - Rate Limiting & Abuse Protection                         │
│   - Anomaly Detection & Behavior Analytics                   │
└─────────────────────────────────────────────────────────────┘
```

## Core Design Principles

1. **Non-Blocking by Default**: Security checks should not add significant latency
2. **Fail-Safe**: Configurable failure modes (fail-open, fail-closed)
3. **Composable**: Chain multiple security hooks together
4. **Observable**: All security decisions generate audit events
5. **Extensible**: Easy to add new security capabilities
6. **MCP-Aware**: Deep inspection of MCP protocol messages

---

## Generic Security Hook Interface

### 1. Rust Trait Definition

```rust
// crates/agentgateway/src/security/mod.rs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Result of a security hook execution
#[derive(Debug, Clone)]
pub enum SecurityDecision {
    /// Allow the request to proceed
    Allow,
    /// Allow with modifications (e.g., sanitized content)
    AllowWithModification(SecurityModification),
    /// Block the request with specific reason
    Deny(SecurityViolation),
    /// Requires additional verification
    RequireAdditionalAuth(String),
}

/// Modifications that can be applied to MCP traffic
#[derive(Debug, Clone)]
pub struct SecurityModification {
    pub modified_request: Option<McpRequest>,
    pub modified_response: Option<McpResponse>,
    pub added_headers: Vec<(String, String)>,
    pub audit_metadata: serde_json::Value,
}

/// Security violation details
#[derive(Debug, Clone, Serialize)]
pub struct SecurityViolation {
    pub rule_id: String,
    pub severity: SecuritySeverity,
    pub threat_type: ThreatType,
    pub description: String,
    pub evidence: serde_json::Value,
    pub recommended_action: String,
}

#[derive(Debug, Clone, Serialize)]
pub enum SecuritySeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub enum ThreatType {
    ToolPoisoning,
    RugPull,
    ToolShadowing,
    ServerSpoofing,
    UnauthorizedAccess,
    PromptInjection,
    CommandInjection,
    DataExfiltration,
    SessionHijacking,
    ContextPoisoning,
    RateLimitExceeded,
    AnomalousBehavior,
}

/// Security context passed to all hooks
#[derive(Debug, Clone)]
pub struct SecurityContext {
    /// Unique correlation ID for request tracing
    pub correlation_id: String,
    /// MCP session information
    pub session: Arc<McpSessionInfo>,
    /// Authenticated user/agent identity
    pub identity: Option<Identity>,
    /// Request metadata
    pub request_metadata: RequestMetadata,
    /// Accumulated security metadata from previous hooks
    pub security_metadata: Arc<std::sync::RwLock<serde_json::Value>>,
}

#[derive(Debug, Clone)]
pub struct McpSessionInfo {
    pub session_id: String,
    pub server_id: String,
    pub server_name: String,
    pub protocol_version: String,
    pub established_at: std::time::SystemTime,
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub user_id: String,
    pub agent_id: Option<String>,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
    pub attributes: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct RequestMetadata {
    pub method: String,
    pub path: String,
    pub headers: http::HeaderMap,
    pub remote_addr: std::net::SocketAddr,
    pub timestamp: std::time::SystemTime,
}

/// MCP-specific request wrapper
#[derive(Debug, Clone)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
    pub id: Option<serde_json::Value>,
}

/// MCP-specific response wrapper
#[derive(Debug, Clone)]
pub struct McpResponse {
    pub jsonrpc: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub id: Option<serde_json::Value>,
}

/// Main security hook trait
#[async_trait]
pub trait SecurityHook: Send + Sync {
    /// Hook identifier (must be unique)
    fn id(&self) -> &str;

    /// Hook display name
    fn name(&self) -> &str;

    /// Hook description
    fn description(&self) -> &str;

    /// Priority (lower = earlier execution)
    fn priority(&self) -> u32;

    /// Whether this hook should run on request path
    fn runs_on_request(&self) -> bool { true }

    /// Whether this hook should run on response path
    fn runs_on_response(&self) -> bool { false }

    /// Initialize the hook (called once at startup)
    async fn initialize(&mut self, config: &HookConfig) -> Result<(), Box<dyn std::error::Error>>;

    /// Inspect and potentially modify/block MCP request
    async fn inspect_request(
        &self,
        context: &SecurityContext,
        request: &McpRequest,
    ) -> Result<SecurityDecision, Box<dyn std::error::Error>>;

    /// Inspect and potentially modify/block MCP response
    async fn inspect_response(
        &self,
        context: &SecurityContext,
        request: &McpRequest,
        response: &McpResponse,
    ) -> Result<SecurityDecision, Box<dyn std::error::Error>>;

    /// Health check
    async fn health_check(&self) -> Result<HealthStatus, Box<dyn std::error::Error>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub enabled: bool,
    pub failure_mode: FailureMode,
    pub timeout_ms: u64,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureMode {
    FailOpen,  // Allow traffic if hook fails
    FailClosed, // Block traffic if hook fails
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub message: String,
}
```

### 2. Hook Registry & Execution Engine

```rust
// crates/agentgateway/src/security/registry.rs

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Central registry for all security hooks
pub struct SecurityHookRegistry {
    hooks: Arc<RwLock<Vec<Arc<dyn SecurityHook>>>>,
    hooks_by_id: Arc<RwLock<HashMap<String, Arc<dyn SecurityHook>>>>,
}

impl SecurityHookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: Arc::new(RwLock::new(Vec::new())),
            hooks_by_id: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new security hook
    pub async fn register(&self, hook: Arc<dyn SecurityHook>) -> Result<(), String> {
        let id = hook.id().to_string();

        // Check for duplicates
        {
            let hooks_by_id = self.hooks_by_id.read().await;
            if hooks_by_id.contains_key(&id) {
                return Err(format!("Hook with id '{}' already registered", id));
            }
        }

        // Insert hook
        {
            let mut hooks = self.hooks.write().await;
            let mut hooks_by_id = self.hooks_by_id.write().await;

            hooks.push(hook.clone());
            hooks_by_id.insert(id, hook);

            // Sort by priority
            hooks.sort_by_key(|h| h.priority());
        }

        Ok(())
    }

    /// Execute all request hooks
    pub async fn execute_request_hooks(
        &self,
        context: &SecurityContext,
        request: &McpRequest,
    ) -> Result<SecurityDecision, Box<dyn std::error::Error>> {
        let hooks = self.hooks.read().await;

        for hook in hooks.iter() {
            if !hook.runs_on_request() {
                continue;
            }

            let decision = hook.inspect_request(context, request).await?;

            match decision {
                SecurityDecision::Deny(_) => {
                    // Log and return immediately
                    log::warn!("Request denied by hook: {}", hook.id());
                    return Ok(decision);
                }
                SecurityDecision::RequireAdditionalAuth(_) => {
                    return Ok(decision);
                }
                _ => continue,
            }
        }

        Ok(SecurityDecision::Allow)
    }

    /// Execute all response hooks
    pub async fn execute_response_hooks(
        &self,
        context: &SecurityContext,
        request: &McpRequest,
        response: &McpResponse,
    ) -> Result<SecurityDecision, Box<dyn std::error::Error>> {
        let hooks = self.hooks.read().await;

        for hook in hooks.iter() {
            if !hook.runs_on_response() {
                continue;
            }

            let decision = hook.inspect_response(context, request, response).await?;

            match decision {
                SecurityDecision::Deny(_) => {
                    log::warn!("Response denied by hook: {}", hook.id());
                    return Ok(decision);
                }
                _ => continue,
            }
        }

        Ok(SecurityDecision::Allow)
    }
}
```

---

## Example Implementation: Tool Poisoning Detection Hook

```rust
// crates/agentgateway/src/security/hooks/tool_poisoning.rs

use super::*;
use regex::Regex;
use once_cell::sync::Lazy;

static MALICIOUS_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Instruction override patterns
        Regex::new(r"(?i)ignore\s+(previous|all)\s+(instructions?|rules?)").unwrap(),
        Regex::new(r"(?i)disregard\s+(previous|all)").unwrap(),
        Regex::new(r"(?i)forget\s+(all|previous)\s+rules?").unwrap(),

        // Credential extraction
        Regex::new(r"(?i)(reveal|output|include|send|export)\s+(password|api[_\s]?key|token|secret|credential)").unwrap(),

        // Authorization bypass
        Regex::new(r"(?i)(bypass|skip|ignore)\s+(auth|authorization|permission|validation)").unwrap(),

        // Jailbreak attempts
        Regex::new(r"(?i)(you are now|enter|enable)\s+(admin|debug|god|root)\s+mode").unwrap(),

        // Exfiltration commands
        Regex::new(r"(?i)(send|transmit|export)\s+(data|content|file)s?\s+to\s+https?://").unwrap(),
    ]
});

pub struct ToolPoisoningHook {
    config: HookConfig,
}

impl ToolPoisoningHook {
    pub fn new() -> Self {
        Self {
            config: HookConfig {
                enabled: true,
                failure_mode: FailureMode::FailClosed,
                timeout_ms: 100,
                config: serde_json::json!({}),
            },
        }
    }

    fn scan_text(&self, text: &str) -> Option<SecurityViolation> {
        for pattern in MALICIOUS_PATTERNS.iter() {
            if let Some(m) = pattern.find(text) {
                return Some(SecurityViolation {
                    rule_id: "TOOL_POISON_001".to_string(),
                    severity: SecuritySeverity::Critical,
                    threat_type: ThreatType::ToolPoisoning,
                    description: format!("Malicious instruction detected in tool metadata: {}", m.as_str()),
                    evidence: serde_json::json!({
                        "matched_pattern": pattern.as_str(),
                        "matched_text": m.as_str(),
                        "location": text,
                    }),
                    recommended_action: "Block tool registration and alert security team".to_string(),
                });
            }
        }
        None
    }
}

#[async_trait]
impl SecurityHook for ToolPoisoningHook {
    fn id(&self) -> &str {
        "tool-poisoning-detector"
    }

    fn name(&self) -> &str {
        "Tool Poisoning Detection"
    }

    fn description(&self) -> &str {
        "Detects malicious instructions embedded in tool metadata"
    }

    fn priority(&self) -> u32 {
        100 // High priority (Tier 1)
    }

    fn runs_on_response(&self) -> bool {
        true // Scan tools/list responses
    }

    async fn initialize(&mut self, config: &HookConfig) -> Result<(), Box<dyn std::error::Error>> {
        self.config = config.clone();
        log::info!("Tool Poisoning Hook initialized");
        Ok(())
    }

    async fn inspect_request(
        &self,
        _context: &SecurityContext,
        _request: &McpRequest,
    ) -> Result<SecurityDecision, Box<dyn std::error::Error>> {
        Ok(SecurityDecision::Allow)
    }

    async fn inspect_response(
        &self,
        context: &SecurityContext,
        request: &McpRequest,
        response: &McpResponse,
    ) -> Result<SecurityDecision, Box<dyn std::error::Error>> {
        // Only scan tools/list responses
        if request.method != "tools/list" {
            return Ok(SecurityDecision::Allow);
        }

        if let Some(result) = &response.result {
            // Parse tools array
            if let Some(tools) = result.get("tools").and_then(|t| t.as_array()) {
                for tool in tools {
                    // Scan tool name
                    if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                        if let Some(violation) = self.scan_text(name) {
                            return Ok(SecurityDecision::Deny(violation));
                        }
                    }

                    // Scan tool description
                    if let Some(desc) = tool.get("description").and_then(|d| d.as_str()) {
                        if let Some(violation) = self.scan_text(desc) {
                            return Ok(SecurityDecision::Deny(violation));
                        }
                    }

                    // Scan input schema descriptions
                    if let Some(schema) = tool.get("inputSchema") {
                        let schema_str = serde_json::to_string(schema)?;
                        if let Some(violation) = self.scan_text(&schema_str) {
                            return Ok(SecurityDecision::Deny(violation));
                        }
                    }
                }
            }
        }

        Ok(SecurityDecision::Allow)
    }

    async fn health_check(&self) -> Result<HealthStatus, Box<dyn std::error::Error>> {
        Ok(HealthStatus {
            healthy: true,
            message: format!("Loaded {} malicious patterns", MALICIOUS_PATTERNS.len()),
        })
    }
}
```

---

## Integration Patterns

### Pattern 1: Native Rust Hook (High Performance)

**Use Case**: Tool Poisoning, Rug Pull, Tool Shadowing

**Pros**:
- Zero latency overhead
- Direct access to MCP protocol structs
- Type-safe

**Cons**:
- Requires Rust knowledge
- Needs recompilation for updates

### Pattern 2: External gRPC Service (ext_authz style)

**Use Case**: Tool-Level Access Control, Context Integrity

**Pros**:
- Language-agnostic
- Can be updated independently
- Shared across multiple gateways

**Cons**:
- Network latency (~5-20ms)
- Requires service management

```yaml
# Configuration example
security:
  hooks:
    - id: rbac-policy-service
      type: external-grpc
      endpoint: grpc://rbac-service:9000
      timeout: 100ms
      failure_mode: fail_closed
      runs_on: request
```

### Pattern 3: Webhook (HTTP Callback)

**Use Case**: DLP, Anomaly Detection, Audit Logging

**Pros**:
- Simple integration
- Language-agnostic
- Easy to develop/test

**Cons**:
- Higher latency (~20-50ms)
- HTTP overhead

```yaml
# Configuration example
security:
  hooks:
    - id: dlp-scanner
      type: webhook
      url: https://dlp-service.internal/scan
      method: POST
      timeout: 200ms
      failure_mode: fail_open
      runs_on: response
      headers:
        Authorization: Bearer ${DLP_API_KEY}
```

### Pattern 4: Wasm Plugin (Future)

**Use Case**: Custom security logic without Rust

**Pros**:
- Near-native performance
- Sandboxed execution
- Dynamic loading

**Cons**:
- Limited ecosystem
- Memory constraints

---

## Configuration Schema

```yaml
# config.yaml
security:
  enabled: true

  # Global settings
  correlation_id_header: X-Correlation-ID
  audit_logging:
    enabled: true
    destination: grpc://audit-service:9001

  # Hook definitions
  hooks:
    # Tier 1: Native Hooks (compiled into gateway)
    - id: tool-poisoning-detector
      type: native
      enabled: true
      priority: 100
      failure_mode: fail_closed
      runs_on: [response]
      config:
        strict_mode: true

    - id: rug-pull-detector
      type: native
      enabled: true
      priority: 101
      failure_mode: fail_closed
      runs_on: [response]
      config:
        baseline_store: redis://localhost:6379/0
        check_interval: 60s

    - id: tool-shadowing-detector
      type: native
      enabled: true
      priority: 102
      failure_mode: fail_closed
      runs_on: [response]
      config:
        namespace_isolation: true

    # Tier 2: External Services
    - id: rbac-enforcer
      type: external-grpc
      endpoint: grpc://rbac-service:9000
      enabled: true
      priority: 200
      timeout: 100ms
      failure_mode: fail_closed
      runs_on: [request]

    - id: content-filter
      type: external-grpc
      endpoint: grpc://content-filter:9001
      enabled: true
      priority: 201
      timeout: 150ms
      failure_mode: fail_closed
      runs_on: [request, response]
      config:
        check_request_body: true
        check_response_body: true
        max_body_size: 1MB

    # Tier 3: Webhooks
    - id: dlp-scanner
      type: webhook
      url: https://dlp-service.internal/scan
      method: POST
      enabled: true
      priority: 300
      timeout: 200ms
      failure_mode: fail_open
      runs_on: [response]
      headers:
        Authorization: Bearer ${DLP_API_KEY}

    - id: anomaly-detector
      type: webhook
      url: https://ml-service.internal/analyze
      method: POST
      enabled: true
      priority: 301
      timeout: 500ms
      failure_mode: fail_open
      runs_on: [request, response]
      config:
        async: true  # Non-blocking analysis
```

---

## Directory Structure

```
crates/agentgateway/src/
├── security/
│   ├── mod.rs                    # Core traits and types
│   ├── registry.rs               # Hook registry and execution engine
│   ├── context.rs                # Security context management
│   ├── hooks/                    # Native hook implementations
│   │   ├── mod.rs
│   │   ├── tool_poisoning.rs     # Tier 1: Tool poisoning detection
│   │   ├── rug_pull.rs           # Tier 1: Rug pull detection
│   │   ├── tool_shadowing.rs     # Tier 1: Tool shadowing prevention
│   │   ├── server_spoofing.rs    # Tier 1: Server whitelisting
│   │   └── content_filter.rs     # Tier 2: Basic content filtering
│   ├── external/                 # External hook integrations
│   │   ├── mod.rs
│   │   ├── grpc_hook.rs          # gRPC ext_authz style
│   │   └── webhook_hook.rs       # HTTP webhook
│   └── middleware.rs             # Axum middleware integration
```

---

## Next Steps

### Phase 1: Core Framework (Week 1-2)
1. Implement core traits and types
2. Build hook registry and execution engine
3. Create security context management
4. Add configuration parsing

### Phase 2: Tier 1 Hooks (Week 3-4)
1. Implement Tool Poisoning Detection
2. Implement Rug Pull Detection
3. Implement Tool Shadowing Prevention
4. Add comprehensive tests

### Phase 3: External Integration (Week 5-6)
1. Build gRPC hook adapter
2. Build webhook hook adapter
3. Create example external services
4. Documentation and examples

### Phase 4: Tier 2 & 3 Hooks (Week 7-8)
1. Implement remaining native hooks
2. Build reference external services
3. Performance testing and optimization
4. Production hardening

---

## Performance Considerations

### Latency Budget

| Hook Type | Target Latency | P99 Latency | Mitigation |
|-----------|---------------|-------------|------------|
| Native Rust | < 1ms | < 5ms | Optimize regex, use lazy statics |
| External gRPC | < 20ms | < 50ms | Connection pooling, circuit breaker |
| Webhook | < 50ms | < 200ms | Async execution, caching |

### Scaling Considerations

1. **Horizontal Scaling**: Hooks should be stateless or use external stores
2. **Caching**: Cache tool baselines, RBAC decisions, etc.
3. **Circuit Breaker**: Automatic failover when external services are down
4. **Rate Limiting**: Protect external services from overload
5. **Async Execution**: Non-critical hooks (audit, analytics) run async

---

## Security & Privacy

1. **Secrets Management**: Hook configs never log secrets
2. **PII Protection**: Automatically mask PII in audit logs
3. **Least Privilege**: Hooks only see data they need
4. **Audit Trail**: All security decisions are logged with correlation IDs
5. **Defense in Depth**: Multiple hooks can detect same threat

---

## Monitoring & Observability

### Metrics

```rust
// Prometheus metrics
security_hooks_total{hook_id, decision}
security_hooks_duration_seconds{hook_id}
security_hooks_errors_total{hook_id, error_type}
security_violations_total{threat_type, severity}
```

### Alerts

1. High rate of security violations
2. Hook execution failures
3. Unusual hook latency
4. External service unavailable

---

## Questions for Discussion

1. **Webhook Authentication**: What auth mechanism for webhooks? (mTLS, API keys, OAuth2)
2. **Policy Storage**: Where to store tool baselines for Rug Pull detection? (Redis, PostgreSQL, File)
3. **Audit Destination**: Where to send audit logs? (Elasticsearch, Splunk, Cloud Logging)
4. **Performance vs Security Trade-off**: Default to fail-open or fail-closed?
5. **Hook Discovery**: Support dynamic hook loading or compile-time only?
