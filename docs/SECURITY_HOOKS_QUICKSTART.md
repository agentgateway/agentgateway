# Security Hooks - Quick Start Guide

## Overview

This guide helps you get started with implementing and using security hooks in the Agent Gateway.

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Client (AI Agent/User)                        │
└────────────────────────────────┬────────────────────────────────────┘
                                 │ MCP Request
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        Agent Gateway (Rust)                           │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │              Request Path (Pre-Routing)                        │ │
│  │                                                                │ │
│  │  1. Rate Limiter (native)         [Priority: 50]             │ │
│  │  2. Tool Poisoning Detector (native)  [Priority: 100]         │ │
│  │  3. Server Whitelisting (native)  [Priority: 103]            │ │
│  │  4. RBAC Enforcer (gRPC)          [Priority: 200]            │ │
│  │  5. Content Filter (gRPC)         [Priority: 201]            │ │
│  │  6. Token Validator (native)      [Priority: 202]            │ │
│  │  7. Context Validator (native)    [Priority: 203]            │ │
│  │                                                                │ │
│  │  ┌──────────────────────────────────────────┐                │ │
│  │  │  Decision Point: Allow/Deny/Modify?     │                │ │
│  │  └──────────────────────────────────────────┘                │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                 │                                     │
│                                 │ If Allow                            │
│                                 ▼                                     │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │                     MCP Router                                 │ │
│  │   (Routes to appropriate MCP server backend)                  │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                 │                                     │
└─────────────────────────────────┼─────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         MCP Server Backend                            │
│              (GitHub, Slack, Database, Filesystem, etc.)             │
└────────────────────────────────┬────────────────────────────────────┘
                                 │ MCP Response
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        Agent Gateway (Rust)                           │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │              Response Path (Post-Processing)                   │ │
│  │                                                                │ │
│  │  1. Tool Poisoning Detector (native)  [Priority: 100]         │ │
│  │  2. Rug Pull Detector (native)        [Priority: 101]         │ │
│  │  3. Tool Shadowing Detector (native)  [Priority: 102]         │ │
│  │  4. Content Filter (gRPC)             [Priority: 201]         │ │
│  │  5. Context Validator (native)        [Priority: 203]         │ │
│  │  6. DLP Scanner (webhook)             [Priority: 300]         │ │
│  │                                                                │ │
│  │  ┌──────────────────────────────────────────┐                │ │
│  │  │  Decision Point: Allow/Deny/Modify?     │                │ │
│  │  └──────────────────────────────────────────┘                │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                 │                                     │
│                                 │ If Allow                            │
│                                 ▼                                     │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │            Async Hooks (Non-Blocking)                          │ │
│  │                                                                │ │
│  │  • Audit Logger (webhook)       [Priority: 900]               │ │
│  │  • Anomaly Detector (webhook)   [Priority: 901]               │ │
│  └────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────┬─────────────────────────────────────┘
                                 │ MCP Response
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         Client (AI Agent/User)                        │
└─────────────────────────────────────────────────────────────────────┘

External Services (Tier 2 & 3):
┌────────────────────┐  ┌────────────────────┐  ┌──────────────────┐
│  RBAC Service      │  │  Content Filter    │  │  DLP Service     │
│  (gRPC:9000)       │  │  (gRPC:9001)       │  │  (HTTP/webhook)  │
└────────────────────┘  └────────────────────┘  └──────────────────┘

┌────────────────────┐  ┌────────────────────┐
│  Audit Service     │  │  ML/Anomaly Svc    │
│  (gRPC/HTTP)       │  │  (HTTP/webhook)    │
└────────────────────┘  └────────────────────┘
```

---

## Implementation Approaches

### Approach 1: Native Rust Hook (Best Performance)

**When to use**: High-priority, performance-critical checks (Tier 1)

**Example**: Tool Poisoning Detection

```bash
# Add to your gateway codebase
crates/agentgateway/src/security/hooks/tool_poisoning.rs
```

**Pros**:
- Zero network latency
- Type-safe, compiled checks
- Direct access to MCP structs

**Cons**:
- Requires Rust knowledge
- Needs gateway recompilation for updates

**Configuration**:
```yaml
security:
  hooks:
    - id: tool-poisoning-detector
      type: native
      enabled: true
      priority: 100
      failure_mode: fail_closed
      runs_on: [response]
```

---

### Approach 2: External gRPC Service (Medium Performance)

**When to use**: Policy-driven checks that need central management (Tier 2)

**Example**: RBAC Enforcement

**Step 1: Define gRPC Service**

```protobuf
// security-service.proto
syntax = "proto3";

package security.v1;

service SecurityService {
  rpc CheckRequest(CheckRequestMessage) returns (CheckResponse);
}

message CheckRequestMessage {
  string correlation_id = 1;
  SecurityContext context = 2;
  McpRequest request = 3;
}

message CheckResponse {
  Decision decision = 1;
  string reason = 2;
  map<string, string> metadata = 3;
}

enum Decision {
  ALLOW = 0;
  DENY = 1;
  REQUIRE_ADDITIONAL_AUTH = 2;
}
```

**Step 2: Implement Service (Python example)**

```python
import grpc
from concurrent import futures
from security_pb2 import Decision, CheckResponse
from security_pb2_grpc import SecurityServiceServicer

class RBACService(SecurityServiceServicer):
    def CheckRequest(self, request, context):
        # Extract user identity
        user_id = request.context.identity.user_id
        tool_name = request.request.params.get("tool_name")

        # Check RBAC policy
        if self.is_authorized(user_id, tool_name):
            return CheckResponse(
                decision=Decision.ALLOW,
                reason="User authorized"
            )
        else:
            return CheckResponse(
                decision=Decision.DENY,
                reason=f"User {user_id} not authorized for tool {tool_name}"
            )

    def is_authorized(self, user_id, tool_name):
        # Your RBAC logic here
        pass

# Start server
server = grpc.server(futures.ThreadPoolExecutor(max_workers=10))
SecurityServiceServicer_grpc.add_SecurityServiceServicer_to_server(
    RBACService(), server
)
server.add_insecure_port('[::]:9000')
server.start()
```

**Step 3: Configure Gateway**

```yaml
security:
  hooks:
    - id: rbac-enforcer
      type: external-grpc
      endpoint: grpc://rbac-service:9000
      enabled: true
      priority: 200
      timeout_ms: 100
      failure_mode: fail_closed
      runs_on: [request]
```

**Pros**:
- Language-agnostic
- Can be updated independently
- Centralized policy management

**Cons**:
- Network latency (5-20ms)
- Requires service deployment

---

### Approach 3: Webhook (HTTP Callback) (Lowest Performance)

**When to use**: Non-critical, async operations (Tier 3)

**Example**: DLP Scanner

**Step 1: Create Webhook Service (Node.js example)**

```javascript
const express = require('express');
const app = express();
app.use(express.json());

app.post('/scan', async (req, res) => {
  const { correlation_id, context, request, response } = req.body;

  // Extract response content
  const content = JSON.stringify(response.result);

  // Scan for sensitive data
  const violations = await scanForSensitiveData(content);

  if (violations.length > 0) {
    return res.json({
      decision: 'DENY',
      violation: {
        rule_id: 'DLP_001',
        severity: 'HIGH',
        threat_type: 'DATA_EXFILTRATION',
        description: `Found ${violations.length} sensitive data patterns`,
        evidence: violations
      }
    });
  }

  return res.json({
    decision: 'ALLOW'
  });
});

async function scanForSensitiveData(content) {
  const patterns = [
    { name: 'credit_card', regex: /\b\d{4}[- ]?\d{4}[- ]?\d{4}[- ]?\d{4}\b/ },
    { name: 'ssn', regex: /\b\d{3}-\d{2}-\d{4}\b/ },
    { name: 'api_key', regex: /\b[A-Za-z0-9]{32,}\b/ },
  ];

  const violations = [];
  for (const pattern of patterns) {
    const matches = content.match(pattern.regex);
    if (matches) {
      violations.push({
        pattern: pattern.name,
        matches: matches.slice(0, 3) // First 3 matches
      });
    }
  }

  return violations;
}

app.listen(8080, () => {
  console.log('DLP service listening on port 8080');
});
```

**Step 2: Configure Gateway**

```yaml
security:
  hooks:
    - id: dlp-scanner
      type: webhook
      url: https://dlp-service.internal/scan
      method: POST
      enabled: true
      priority: 300
      timeout_ms: 200
      failure_mode: fail_open
      runs_on: [response]
      headers:
        Authorization: Bearer ${DLP_API_KEY}
```

**Webhook Request Format**:
```json
{
  "correlation_id": "req-123-abc-456",
  "context": {
    "session": {...},
    "identity": {...},
    "request_metadata": {...}
  },
  "request": {
    "jsonrpc": "2.0",
    "method": "tools/call",
    "params": {...}
  },
  "response": {
    "jsonrpc": "2.0",
    "result": {...}
  }
}
```

**Webhook Response Format**:
```json
{
  "decision": "ALLOW|DENY|ALLOW_WITH_MODIFICATION",
  "modification": {
    "modified_response": {...},
    "audit_metadata": {...}
  },
  "violation": {
    "rule_id": "DLP_001",
    "severity": "HIGH",
    "threat_type": "DATA_EXFILTRATION",
    "description": "...",
    "evidence": {...}
  }
}
```

**Pros**:
- Simple HTTP interface
- Language-agnostic
- Easy to develop and test

**Cons**:
- Higher latency (20-50ms)
- HTTP overhead

---

## Quick Start: Adding Your First Hook

### Option A: Native Rust Hook

1. **Create hook file**:
```bash
touch crates/agentgateway/src/security/hooks/my_custom_hook.rs
```

2. **Implement the trait**:
```rust
use crate::security::*;

pub struct MyCustomHook {
    config: HookConfig,
}

#[async_trait]
impl SecurityHook for MyCustomHook {
    fn id(&self) -> &str {
        "my-custom-hook"
    }

    fn name(&self) -> &str {
        "My Custom Security Check"
    }

    fn description(&self) -> &str {
        "Detects XYZ security threat"
    }

    fn priority(&self) -> u32 {
        100
    }

    async fn initialize(&mut self, config: &HookConfig) -> Result<(), Box<dyn std::error::Error>> {
        self.config = config.clone();
        Ok(())
    }

    async fn inspect_request(
        &self,
        context: &SecurityContext,
        request: &McpRequest,
    ) -> Result<SecurityDecision, Box<dyn std::error::Error>> {
        // Your security logic here
        if is_threat_detected(request) {
            return Ok(SecurityDecision::Deny(SecurityViolation {
                rule_id: "CUSTOM_001".to_string(),
                severity: SecuritySeverity::High,
                threat_type: ThreatType::UnauthorizedAccess,
                description: "Threat detected".to_string(),
                evidence: serde_json::json!({"request": request}),
                recommended_action: "Block and alert".to_string(),
            }));
        }

        Ok(SecurityDecision::Allow)
    }

    async fn inspect_response(
        &self,
        _context: &SecurityContext,
        _request: &McpRequest,
        _response: &McpResponse,
    ) -> Result<SecurityDecision, Box<dyn std::error::Error>> {
        Ok(SecurityDecision::Allow)
    }

    async fn health_check(&self) -> Result<HealthStatus, Box<dyn std::error::Error>> {
        Ok(HealthStatus {
            healthy: true,
            message: "Hook is healthy".to_string(),
        })
    }
}

fn is_threat_detected(request: &McpRequest) -> bool {
    // Your detection logic
    false
}
```

3. **Register hook**:
```rust
// In your gateway initialization code
let registry = SecurityHookRegistry::new();
registry.register(Arc::new(MyCustomHook::new())).await?;
```

4. **Configure in YAML**:
```yaml
security:
  hooks:
    - id: my-custom-hook
      type: native
      enabled: true
      priority: 100
      failure_mode: fail_closed
      runs_on: [request]
```

### Option B: External Service (Python)

1. **Create service**:
```python
from flask import Flask, request, jsonify

app = Flask(__name__)

@app.route('/check', methods=['POST'])
def check_security():
    data = request.json
    correlation_id = data['correlation_id']
    mcp_request = data['request']

    # Your security logic
    if is_threat(mcp_request):
        return jsonify({
            'decision': 'DENY',
            'violation': {
                'rule_id': 'CUSTOM_001',
                'severity': 'HIGH',
                'threat_type': 'UNAUTHORIZED_ACCESS',
                'description': 'Threat detected',
                'evidence': mcp_request
            }
        })

    return jsonify({'decision': 'ALLOW'})

def is_threat(mcp_request):
    # Your logic
    return False

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=8080)
```

2. **Deploy service**:
```bash
docker build -t my-security-service .
docker run -p 8080:8080 my-security-service
```

3. **Configure gateway**:
```yaml
security:
  hooks:
    - id: my-custom-hook
      type: webhook
      url: http://my-security-service:8080/check
      method: POST
      enabled: true
      priority: 300
      timeout_ms: 200
      failure_mode: fail_closed
      runs_on: [request]
```

---

## Testing Your Hooks

### Unit Tests (Rust)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_hook_detects_threat() {
        let hook = MyCustomHook::new();

        let request = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: serde_json::json!({"malicious": true}),
            id: Some(serde_json::json!(1)),
        };

        let context = create_test_context();

        let decision = hook.inspect_request(&context, &request).await.unwrap();

        match decision {
            SecurityDecision::Deny(violation) => {
                assert_eq!(violation.rule_id, "CUSTOM_001");
            }
            _ => panic!("Expected Deny decision"),
        }
    }
}
```

### Integration Tests

```bash
# Test with curl
curl -X POST http://localhost:8080/mcp/tools/call \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer test-token" \
  -d '{
    "jsonrpc": "2.0",
    "method": "tools/call",
    "params": {
      "name": "malicious_tool",
      "arguments": {}
    },
    "id": 1
  }'
```

---

## Monitoring & Debugging

### Enable Debug Logging

```yaml
logging:
  level: debug
  components:
    - security
```

### View Metrics

```bash
# Prometheus metrics
curl http://localhost:9090/metrics | grep security_
```

### Check Hook Health

```bash
curl http://localhost:15000/health/security-hooks
```

---

## Common Patterns

### Pattern 1: Caching Results

```rust
use std::collections::HashMap;
use std::sync::RwLock;

struct CachedHook {
    cache: Arc<RwLock<HashMap<String, SecurityDecision>>>,
}

impl CachedHook {
    async fn inspect_request(&self, context: &SecurityContext, request: &McpRequest)
        -> Result<SecurityDecision, Box<dyn std::error::Error>>
    {
        let cache_key = format!("{}:{}", context.identity.user_id, request.method);

        // Check cache
        {
            let cache = self.cache.read().unwrap();
            if let Some(decision) = cache.get(&cache_key) {
                return Ok(decision.clone());
            }
        }

        // Perform check
        let decision = self.do_security_check(request).await?;

        // Cache result
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(cache_key, decision.clone());
        }

        Ok(decision)
    }
}
```

### Pattern 2: Circuit Breaker for External Services

```rust
use std::sync::atomic::{AtomicU32, Ordering};

struct ExternalHook {
    failure_count: Arc<AtomicU32>,
    circuit_open: Arc<AtomicBool>,
}

impl ExternalHook {
    async fn call_external_service(&self) -> Result<SecurityDecision, Box<dyn std::error::Error>> {
        // Check circuit breaker
        if self.circuit_open.load(Ordering::Relaxed) {
            log::warn!("Circuit breaker open, failing fast");
            return Ok(SecurityDecision::Allow); // Fail open
        }

        match self.make_request().await {
            Ok(decision) => {
                self.failure_count.store(0, Ordering::Relaxed);
                Ok(decision)
            }
            Err(e) => {
                let failures = self.failure_count.fetch_add(1, Ordering::Relaxed);
                if failures > 5 {
                    log::error!("Opening circuit breaker after {} failures", failures);
                    self.circuit_open.store(true, Ordering::Relaxed);
                }
                Err(e)
            }
        }
    }
}
```

---

## Next Steps

1. Review [SECURITY_HOOKS_DESIGN.md](./SECURITY_HOOKS_DESIGN.md) for detailed architecture
2. Check [security-hooks-config-example.yaml](./security-hooks-config-example.yaml) for configuration options
3. Implement your first Tier 1 hook (Tool Poisoning Detection)
4. Set up monitoring and alerting
5. Deploy to staging environment for testing

---

## FAQ

**Q: Can I mix native and external hooks?**
A: Yes! The framework supports all three types (native, gRPC, webhook) simultaneously.

**Q: What happens if a hook times out?**
A: Depends on `failure_mode`:
- `fail_closed`: Request is blocked
- `fail_open`: Request is allowed (logged as warning)

**Q: Can hooks modify requests/responses?**
A: Yes, return `SecurityDecision::AllowWithModification(...)` with the modified content.

**Q: How do I debug a hook that's blocking legitimate traffic?**
A:
1. Check audit logs with correlation ID
2. Review violation evidence
3. Temporarily set `failure_mode: fail_open` for that hook
4. Add more specific detection logic

**Q: Can hooks call each other?**
A: No, hooks execute independently in priority order. Share data via `security_metadata` in context.

**Q: What's the performance impact?**
A:
- Native hooks: < 1ms per hook
- gRPC: 5-20ms per call
- Webhooks: 20-50ms per call
