# Feature Request: Forward Authorization Headers to HTTP MCP Backends

## Summary

The `backendAuth: passthrough` policy does not forward JWT tokens to HTTP-based MCP backends, preventing MCP servers from authenticating with downstream services. Testing confirms that neither HTTP headers nor Rust `Extension<Claims>` are forwarded to HTTP MCP backends.

## Current Behavior

When using `mcpAuthentication` with `backendAuth: passthrough`:
- ✅ AgentGateway successfully validates JWT tokens
- ✅ JWT claims are available internally (visible in logs: `jwt.sub=auth0|...`)
- ❌ Authorization header is NOT forwarded to HTTP MCP backends
- ❌ Extension<Claims> is NOT forwarded to HTTP MCP backends (even Rust)
- ❌ HTTP MCP servers receive requests without authentication credentials

## Expected Behavior

HTTP-based MCP backends should receive either:

**Option A: HTTP Header Forwarding** (Preferred for cross-language support)
```http
POST /mcp HTTP/1.1
Authorization: Bearer eyJhbGc...
Content-Type: application/json
```

**Option B: Extension Forwarding** (Rust-only)
```rust
Extension<Claims> {
    sub: Some("auth0|66905ae19519996efc385d38"),
    iss: Some("https://sandbox.grainger-development.auth0app.com/"),
    ...
}
```

## Evidence

### Test Setup

We tested with both Python and Rust HTTP MCP servers to isolate the issue:

1. **Python HTTP MCP server** (port 8009) - Cannot access Rust extensions
2. **Rust HTTP MCP server** (port 8010) - Can access `Extension<Claims>`

### Configuration

```yaml
binds:
- listeners:
  - routes:
    - backends:
      - mcp:
          targets:
          - name: sample-commerce
            mcp:
              host: http://localhost:8010/mcp  # HTTP-based backend
      matches:
      - path:
          exact: /commerce/mcp
      policies:
        cors:
          allowHeaders:
          - authorization
          allowOrigins: ['*']
        
        mcpAuthentication:
          issuer: https://sandbox.grainger-development.auth0app.com/
          audiences:
          - http://localhost:3000/commerce/mcp
          - api://commerce
          jwks:
            url: https://sandbox.grainger-development.auth0app.com/.well-known/jwks.json
          provider:
            auth0: {}
          resourceMetadata:
            resource: http://localhost:3000/commerce/mcp
            scopesSupported: [openid, profile, email]
            bearerMethodsSupported: [header, body, query]
        
        backendAuth:
          passthrough: {}
```

### Test Results

#### Python HTTP MCP Server
```python
# Headers received by Python MCP server:
{
  'content-type': 'application/json',
  'mcp-session-id': '...',
  'user-agent': 'python-httpx/0.28.1',
  # ❌ NO 'authorization' header
}
```

#### Rust HTTP MCP Server
```rust
// Rust MCP server logs:
ℹ No JWT claims (public request)
ℹ No Authorization header
ERROR: get_cart called without JWT claims!
```

Even with explicit `Extension<Claims>` extraction:
```rust
async fn handle_mcp(
    claims: Option<Extension<Claims>>,  // ❌ Always None
    Json(req): Json<McpRequest>,
) {
    if let Some(Extension(claims_data)) = &claims {
        // Never reached
    }
}
```

#### AgentGateway Logs (Success)
```
2025-12-14T18:05:18.602564Z info request 
  jwt.sub=auth0|66905ae19519996efc385d38  ✅ Token validated
  protocol=mcp 
  http.status=200  ✅ Request succeeded
```

### Conclusion

AgentGateway successfully:
- ✅ Validates JWT tokens
- ✅ Processes MCP requests
- ✅ Returns 200 responses

But does NOT:
- ❌ Forward Authorization header to HTTP backends
- ❌ Forward Extension<Claims> to HTTP backends
- ❌ Provide any authentication context to HTTP MCP servers

## Related Commits

Investigation found these related commits:
- `8215996` (Aug 2025): "mcp: add support for passthrough backend auth"
- `0bf04c9` (Oct 2025): "mcp: properly apply backend policies to passthrough"
- `d2f2056` (Nov 2025): "feat: Support MCP Authn when configured by xDS"

These commits appear to implement token forwarding for **stdio MCP backends**, but testing shows HTTP MCP backends receive no authentication context.

## Proposed Solution

Implement HTTP header forwarding for HTTP-based MCP backends when `backendAuth: passthrough` is configured:

```rust
// Pseudo-code for the fix
async fn forward_to_http_mcp_backend(
    req: &McpRequest,
    claims: &Option<Claims>,
    original_headers: &HeaderMap,
) {
    let mut backend_request = build_mcp_request(req);
    
    // NEW: Forward Authorization header from original request
    if let Some(auth_header) = original_headers.get("authorization") {
        backend_request.headers_mut()
            .insert("authorization", auth_header.clone());
    }
    
    // Send to HTTP MCP backend
    send_to_backend(backend_request).await
}
```

### Alternative: Configuration Option

If preserving current behavior is needed:

```yaml
backendAuth:
  passthrough:
    forwardHeaders: true  # New option for HTTP backends
```

## Use Case

This is essential for the MCP Authorization pattern where:

1. **User** authenticates with OAuth2/OIDC provider (Auth0, Keycloak, etc.)
2. **MCP Client** sends requests with `Authorization: Bearer <token>`
3. **AgentGateway** validates the token against OAuth provider
4. **MCP Server** needs the token to:
   - Authenticate with downstream microservices
   - Enforce user-level permissions
   - Audit user actions
   - Pass user context to backend APIs

### Real-World Example: E-commerce MCP Server

```
User → MCP Client (authenticated)
  ↓ Authorization: Bearer <user_token>
AgentGateway (validates token) ✅
  ↓ ❌ NO token forwarded
MCP Server (needs token!)
  ↓ Authorization: Bearer <user_token> ← NEEDED!
Backend Services:
  - Shopping Cart API (user-specific data)
  - Order API (user's order history)
  - Payment API (user's payment methods)
```

Without token forwarding, MCP servers cannot:
- ❌ Authenticate to backend services
- ❌ Access user-specific resources
- ❌ Maintain user context
- ❌ Audit user actions

## Workarounds (Suboptimal)

### 1. Trust Localhost
Backend services accept requests from localhost without tokens.

**Issues:**
- Removes defense-in-depth
- No user context
- Cannot use shared backend services
- Not suitable for production

### 2. Service Account Token
MCP server uses separate service account credentials.

**Issues:**
- Loses user context (all requests appear as service account)
- Cannot enforce user-level permissions
- Additional token management complexity

### 3. Skip AgentGateway
Use MCP server with built-in authentication.

**Issues:**
- Loses all AgentGateway benefits (routing, rate limiting, observability)
- Defeats the purpose

## Testing

We have a complete test setup ready to validate any fix:

### Test Servers
1. **Python HTTP MCP Server** (`/sample-commerce/mcp-server-native/`)
   - Logs all received headers
   - 12 tools (3 public, 9 protected)

2. **Rust HTTP MCP Server** (`/sample-commerce/mcp-server-rust/`)
   - Tests `Extension<Claims>` extraction
   - Same 12 tools

### Test Client
- OAuth2 device flow with Auth0
- Sends authenticated requests
- Validates responses

### Success Criteria

After fix, Rust server should log:
```
✓ JWT Claims received from AgentGateway!
  Subject: Some("auth0|66905ae19519996efc385d38")
```

OR Python server should log:
```
✓ Authorization header present: Bearer eyJhbGc...
```

AND protected tools should return actual data instead of 401 errors.

## Questions

1. Is HTTP header forwarding the intended approach, or should we use a different mechanism?

2. Should this behavior be opt-in (new config option) or automatic with `backendAuth: passthrough`?

3. Are there security considerations for forwarding headers to HTTP backends vs. stdio backends?

4. Is there existing functionality we missed that should enable this?

## Environment

- **AgentGateway Version:** v0.10.5-67-g76b0afc (latest main branch)
- **MCP Protocol:** Streamable HTTP (spec 2024-11-05)
- **Authentication:** Auth0 with JWT tokens
- **Backend Types Tested:** Python HTTP, Rust HTTP

## References

- **Issue Documentation:** `AGENTGATEWAY_AUTH_FORWARDING_ISSUE.md` (detailed diagnostic info)
- **Test Servers:**
  - Python: `/sample-commerce/mcp-server-native/`
  - Rust: `/sample-commerce/mcp-server-rust/`
- **Configuration:** `sample-commerce-auth0.yaml`
- **MCP Authorization Spec:** https://spec.modelcontextprotocol.io/specification/2024-11-05/authorization/

## Impact

**Severity:** High - Blocks real-world MCP deployments with authentication

**Affected Users:**
- Anyone using HTTP-based MCP servers (Python, Node.js, Go, etc.)
- Anyone needing user context in MCP tools
- Anyone with microservice architectures requiring authentication

**Current Limitation:**
- Only stdio MCP backends may work (untested but suspected)
- HTTP MCP backends completely broken for authenticated scenarios

## Proposed PR Changes

We can contribute the fix if guidance is provided on:
1. Preferred implementation location (which module/file)
2. Whether to use HTTP headers or another mechanism
3. Configuration approach (automatic vs. opt-in)
4. Test requirements

---

**We're ready to help implement and test this feature!** Please let us know the preferred approach and we can submit a PR.

**Contact:** [Your GitHub handle / Email]  
**Test Environment:** Fully functional, reproducible test case available
