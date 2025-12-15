# AgentGateway Authorization Header Forwarding Issue

## Issue Summary

When using AgentGateway with MCP (Model Context Protocol) over HTTP backends, the `Authorization` header containing validated JWT tokens is **not being forwarded** to the backend MCP server, even when using `backendAuth: passthrough: {}` policy.

This prevents HTTP-based MCP servers from accessing user authentication tokens that were validated by AgentGateway, breaking the intended authentication flow where:
1. AgentGateway validates user tokens (Auth0/OAuth2)
2. AgentGateway forwards the token to the MCP server
3. MCP server uses the token to authenticate to backend API services

## Environment

- **AgentGateway Version:** Latest (as of December 2024)
- **MCP Protocol:** Streamable HTTP (MCP spec 2024-11-05)
- **Authentication:** Auth0 with JWT tokens
- **Backend Type:** HTTP-based MCP server (not stdio)

## Expected Behavior

When a request with a valid `Authorization: Bearer <token>` header is sent to AgentGateway:

1. AgentGateway's `mcpAuthentication` policy validates the JWT token
2. AgentGateway forwards the request to the backend MCP server at `http://localhost:8009/mcp`
3. **The `Authorization` header should be included in the forwarded request**
4. The backend MCP server can extract the token and use it to authenticate with downstream services

## Actual Behavior

The `Authorization` header is **NOT forwarded** to the backend MCP server:

```
# Request received by backend MCP server (missing Authorization header):
{
  'content-type': 'application/json',
  'accept': 'text/event-stream, application/json',
  'accept-encoding': 'gzip, deflate, zstd',
  'mcp-session-id': 'c4579a75-8624-4d59-aaed-0513f1f2c1d2',
  'user-agent': 'python-httpx/0.28.1',
  'host': 'localhost:8009',
  'content-length': '90'
  # ❌ NO 'authorization' header
}
```

## Configuration

### AgentGateway Config (`sample-commerce-auth0.yaml`)

```yaml
binds:
- listeners:
  - routes:
    - backends:
      - mcp:
          targets:
          - name: sample-commerce
            mcp:
              host: http://localhost:8009/mcp
      matches:
      - path:
          exact: /commerce/mcp
      - path:
          exact: /.well-known/oauth-protected-resource/commerce/mcp
      - path:
          exact: /.well-known/oauth-authorization-server/commerce/mcp
      policies:
        cors:
          allowHeaders:
          - mcp-protocol-version
          - content-type
          - authorization
          - accept
          allowOrigins:
          - '*'
          allowMethods:
          - GET
          - POST
          - OPTIONS
        
        # MCP Authentication - Validates JWTs and provides OAuth metadata
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
            scopesSupported:
            - openid
            - profile
            - email
            - offline_access
            - read:products
            - write:cart
            - write:orders
            bearerMethodsSupported:
            - header
            - body
            - query
            resourceDocumentation: http://localhost:3000/commerce/docs
            resourcePolicyUri: http://localhost:3000/commerce/policies
        
        # This should forward the Authorization header but doesn't work with mcpAuthentication
        backendAuth:
          passthrough: {}

  port: 3000
```

### Test Flow

1. **Client authenticates** via OAuth device flow and obtains JWT token
2. **Client sends request** to AgentGateway with `Authorization: Bearer <token>`
3. **AgentGateway validates** the token successfully (logs show `jwt.sub` present)
4. **AgentGateway forwards** request to backend MCP server
5. **Backend MCP server receives** request WITHOUT `Authorization` header ❌

## Diagnostic Evidence

### 1. Client Successfully Authenticates

```
✓ Successfully authenticated!
  Token expires in: 86400 seconds

🔍 Token Claims:
   Issuer: https://sandbox.grainger-development.auth0app.com/
   Audience: ['api://commerce', 'https://sandbox.grainger-development.auth0app.com/userinfo']
   Subject: auth0|66905ae19519996efc385d38
   Scopes: openid profile email
```

### 2. Client Sends Authorization Header

Test client code confirms header is sent:

```python
headers = {
    "Content-Type": "application/json",
    "Accept": "application/json, text/event-stream"
}

if self.access_token:
    headers["Authorization"] = f"Bearer {self.access_token}"  # ✅ Header added

response = await client.post(
    "http://localhost:3000/commerce/mcp",
    headers=headers,
    json=payload
)
```

### 3. AgentGateway Validates Token

AgentGateway logs show successful validation:

```
2025-12-14T17:38:19.777568Z info request 
  gateway=default/bind/3000 
  listener=listener0 
  route=default/route0 
  jwt.sub=auth0|66905ae19519996efc385d38  # ✅ Token validated
  protocol=mcp 
  mcp.method=tools/call 
  http.status=200
```

### 4. Backend MCP Server Does NOT Receive Authorization Header

Backend server logs show missing header:

```python
# Native MCP server logs
2025-12-14 12:38:19,761 - __main__ - INFO - Received headers: {
  'content-type': 'application/json',
  'accept': 'text/event-stream, application/json',
  'accept-encoding': 'gzip, deflate, zstd',
  'mcp-session-id': '4964d1a1-c16a-429a-a831-835b422a9855',
  'user-agent': 'python-httpx/0.28.1',
  'host': 'localhost:8009',
  'content-length': '90'
  # ❌ NO 'authorization' header
}

2025-12-14 12:38:19,761 - __main__ - INFO - ℹ Request without authentication (public tool)
```

## Attempts to Resolve

### Attempt 1: Using `backendAuth: passthrough` at Route Level
**Result:** Authorization header not forwarded

### Attempt 2: Using `jwtAuth` + `mcpAuthentication` + `backendAuth`
**Result:** AgentGateway logs warning: `"MCP backend authentication configured but JWT token already validated and stripped by Gateway or Route level policy"`

This indicates `jwtAuth` consumes/strips the token.

### Attempt 3: Using Only `mcpAuthentication` + `backendAuth: passthrough`
**Result:** Authorization header still not forwarded

### Attempt 4: Moving `backendAuth` to Backend Level
**Result:** Configuration error - MCP backends don't support policies at that level:
```
Error: binds[0].listeners[0].routes[0].backends[0]: unknown field `policies`, expected one of `targets`, `statefulMode`, `prefixMode`
```

## Analysis

### Working Examples vs Our Use Case

The `examples/mcp-authentication/config.yaml` in AgentGateway repo shows MCP authentication examples, but:

1. **All examples use stdio-based MCP servers** (local processes):
```yaml
- mcp:
    targets:
    - name: everything
      stdio:
        args: ['@modelcontextprotocol/server-everything']
        cmd: npx
```

2. **The one HTTP MCP example** (Scenario B) proxies to an external service but doesn't demonstrate token forwarding:
```yaml
- mcp:
    targets:
    - mcp:
        host: https://mcpbin.is.solo.io/remote/mcp
      name: mcpbin
```

3. **No example shows `backendAuth: passthrough` with MCP backends**

### Investigation of Recent Changes

After reviewing AgentGateway commit history, **token forwarding support was added but with a critical limitation**:

#### Commits Related to MCP Auth Forwarding:
- **August 2025** - Commit `8215996`: "mcp: add support for passthrough backend auth"
- **October 2025** - Commit `0bf04c9`: "mcp: properly apply backend policies to passthrough"
- **November 2025** - Commit `d2f2056`: "feat: Support MCP Authn when configured by xDS"

#### The Implementation Problem:

Examining the code changes in commit `8215996`, the implementation shows:

```rust
// From crates/agentgateway/src/mcp/openapi/mod.rs
async fn call_tool(
    &self,
    name: &str,
    args: Option<JsonObject>,
    user_headers: &HeaderMap,
    claims: Option<Claims>,  // JWT claims added here
) -> Result<serde_json::Value, anyhow::Error> {
    // ...
    if let Some(claims) = claims.as_ref() {
        request.extensions_mut().insert(claims.clone());  // ❌ Added to Rust extensions!
    }
    // ...
}
```

**The Problem:** JWT claims are passed via Rust `request.extensions()`, which is an internal Rust mechanism. This works for:
- ✅ **Rust-based MCP servers** that can access `request.extensions()`
- ✅ **stdio-based MCP servers** (where the implementation differs)

But it does **NOT work for:**
- ❌ **HTTP-based MCP servers** in Python/Node.js/Go that need the `Authorization` HTTP header
- ❌ **Any MCP server that doesn't run in the same Rust process**

### Suspected Root Cause

The `mcpAuthentication` + `backendAuth: passthrough` implementation:
- ✅ Validates JWT tokens correctly
- ✅ Exposes OAuth metadata endpoints
- ✅ Populates `jwt.*` variables for authorization rules
- ✅ **Passes claims to Rust-internal MCP implementations**
- ❌ **Does NOT forward the `Authorization` HTTP header to external HTTP-based MCP servers**

This is why:
1. `mcpAuthentication` was designed primarily for stdio MCP servers (which run in-process)
2. Token forwarding logic exists but uses Rust extensions instead of HTTP headers
3. HTTP MCP backends need the actual HTTP `Authorization` header, which isn't being forwarded

## Impact

This issue prevents the following architecture pattern:

```
┌──────────────┐
│  MCP Client  │
│   (with JWT) │
└──────┬───────┘
       │ Authorization: Bearer <token>
       v
┌──────────────────┐
│  AgentGateway    │
│  - Validates JWT │ ✅ Works
│  - Should forward│ ❌ Doesn't work (CONFIRMED via Rust + Python tests)
└──────┬───────────┘
       │ NO Authorization header OR Extension<Claims>!
       v
┌──────────────────┐
│  MCP Server      │
│  (HTTP-based)    │ ❌ Can't authenticate to backend services
│  Python/Rust/etc │    (Tested with both - neither receives tokens)
└──────┬───────────┘
       │ Authorization: Bearer <token> (needed)
       v
┌──────────────────┐
│  Backend APIs    │
│  (require auth)  │
└──────────────────┘
```

**Confirmed via testing (Dec 14, 2024):**
- ❌ Python HTTP MCP server: No headers, no tokens
- ❌ Rust HTTP MCP server: No Extension<Claims>, no headers
- ✅ AgentGateway validates tokens correctly
- ❌ But doesn't forward them to HTTP MCP backends

## Workarounds (Suboptimal)

### Workaround 1: Trust Localhost
Configure backend services to trust requests from localhost without tokens.

**Pros:** Works immediately  
**Cons:** 
- Removes defense-in-depth
- Breaks other authentication patterns that need end-to-end token flow
- Not suitable for production

### Workaround 2: Service Account Token
MCP server uses a separate service account token to call backend services.

**Pros:** Maintains security  
**Cons:**
- Loses user context (all requests appear as service account)
- Additional token management complexity
- Not the intended MCP Authorization spec pattern

### Workaround 3: Bypass AgentGateway
Use direct MCP server with built-in authentication.

**Pros:** Works as expected  
**Cons:** 
- Loses all AgentGateway benefits (routing, rate limiting, etc.)
- Defeats the purpose of using AgentGateway

## Requested Fix

### Current State (As of December 2024)

Recent commits show that token forwarding **was added** but only works for **Rust-internal MCP servers**:

- **Commit 8215996 (Aug 2025):** Added passthrough backend auth support
- **Commit 0bf04c9 (Oct 2025):** Fixed backend policy application
- **Implementation:** JWT claims passed via Rust `request.extensions()`, not HTTP headers

This works for stdio/Rust MCP servers but **NOT for HTTP-based MCP servers** in other languages.

### Needed Enhancement

The AgentGateway needs to forward the actual `Authorization` HTTP header (not just internal Rust extensions) to HTTP-based MCP backends.

### Option A: Make `mcpAuthentication` Forward Headers

Modify `mcpAuthentication` policy to forward the `Authorization` header to HTTP-based MCP backends when `backendAuth: passthrough` is configured:

```yaml
policies:
  mcpAuthentication:
    # ... existing config ...
  backendAuth:
    passthrough: {}  # Should forward Authorization header to HTTP MCP backends
```

### Option B: Support `jwtAuth` Without Stripping Token

Allow `jwtAuth` to validate tokens without consuming them, so they're available for `backendAuth: passthrough`:

```yaml
policies:
  jwtAuth:
    # ... existing config ...
    stripToken: false  # New option to keep Authorization header
  backendAuth:
    passthrough: {}
```

### Option C: Add MCP-Specific Token Forwarding

Add explicit configuration for MCP backends to forward tokens:

```yaml
- mcp:
    targets:
    - name: sample-commerce
      mcp:
        host: http://localhost:8009/mcp
        forwardAuth: true  # New option
```

## Expected Behavior After Fix

After testing with a Rust HTTP MCP server, we confirmed that **NO tokens are forwarded** to HTTP MCP backends (Rust or Python). The Rust server logs show:

```
ℹ No JWT claims (public request)
ℹ No Authorization header
ERROR: get_cart called without JWT claims!
```

This proves `backendAuth: passthrough` does NOT work for HTTP MCP backends at all.

### What Backend Should Receive After Fix

```python
# Backend MCP server should receive:
{
  'content-type': 'application/json',
  'accept': 'text/event-stream, application/json',
  'authorization': 'Bearer eyJhbGc...',  # ✅ Token forwarded!
  'mcp-session-id': '...',
  'user-agent': 'python-httpx/0.28.1',
  'host': 'localhost:8010'
}
```

**OR** for Rust servers:
```rust
Extension<Claims> {  // ✅ Claims forwarded!
    sub: Some("auth0|..."),
    iss: Some("https://..."),
    scope: Some("openid profile email")
}
```

## Test Case for Validation

We have a complete test setup ready to validate any fix:

1. **Test client** that performs OAuth device flow and sends authenticated requests
2. **AgentGateway** configured with Auth0 and `mcpAuthentication`
3. **Native MCP server** that logs all received headers
4. **Backend services** that validate Auth0 tokens

Once fixed, running the test should show:
```
✓ Token validated by AgentGateway
✓ Token received by MCP server  
✓ Token used to authenticate with backend services
✓ Protected tools return actual data (not 401)
```

## Additional Context

- **MCP Authorization Spec:** https://spec.modelcontextprotocol.io/specification/2024-11-05/authorization/
- **Use Case:** E-commerce MCP server with public (product listing) and protected (cart, orders) tools
- **Authentication Flow:** Standard OAuth2/OIDC with Auth0
- **Backend Services:** Microservices requiring JWT validation

## Files for Reference

- AgentGateway config: `/Users/xvxy006/Pictures/Git_Repos/agentgateway/sample-commerce-auth0.yaml`
- Native MCP server: `/Users/xvxy006/Pictures/Git_Repos/sample-commerce/mcp-server-native/mcp_server_native.py`
- Test client: `/Users/xvxy006/Pictures/Git_Repos/sample-commerce/test_agentgateway_auth.py`

---

## Questions for AgentGateway Team

1. ~~Is token forwarding to HTTP-based MCP backends supported?~~ **UPDATE:** Token forwarding exists but only via Rust `request.extensions()`, not HTTP headers.

2. Can the `backendAuth: passthrough` be enhanced to forward the `Authorization` HTTP header (not just Rust extensions) for HTTP-based MCP backends?

3. Is there a specific reason why HTTP header forwarding wasn't implemented alongside the Rust extensions approach?

4. What is the recommended pattern for **HTTP-based MCP servers** (Python/Node.js/Go) that need to authenticate with downstream services using the user's token?

5. Are there any working examples of HTTP MCP backends (not stdio) receiving forwarded authentication credentials?

6. Is support for HTTP header forwarding to HTTP-based MCP backends on the roadmap?

### Reference Commits

- `8215996` - "mcp: add support for passthrough backend auth" (Aug 2025)
- `0bf04c9` - "mcp: properly apply backend policies to passthrough" (Oct 2025)  
- `d2f2056` - "feat: Support MCP Authn when configured by xDS" (Nov 2025)

These commits added token forwarding via Rust extensions but not HTTP headers.

---

**Contact:** [Your contact information]  
**Date:** December 14, 2024  
**AgentGateway Repository:** https://github.com/solo-io/agentgateway
