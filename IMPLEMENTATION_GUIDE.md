# Implementation Guide: Fix HTTP Authorization Header Forwarding

## Problem Analysis

### Root Cause

**File:** `crates/agentgateway/src/mcp/router.rs`  
**Line:** 176

```rust
match auth.jwt_validator.validate_claims(bearer.token()) {
    Ok(claims) => {
        ctx.with_jwt(&claims);
        req.headers_mut().remove(http::header::AUTHORIZATION);  // ❌ PROBLEM!
        req.extensions_mut().insert(claims);
    },
```

The Authorization header is **removed** after validation and never restored, even when `backendAuth: passthrough` is configured.

## Implementation Plan

### Option A: Simple Fix (Recommended)

Don't remove the Authorization header when `backendAuth: passthrough` is configured.

### Option B: Restore Header Before Forwarding

Remove it initially (current behavior) but restore it before sending to HTTP backends.

## Step-by-Step Implementation

### 1. Understand the Flow

```
Client Request with Auth Header
    ↓
router.rs:168 - Extract Authorization header
    ↓
router.rs:172 - Validate JWT
    ↓
router.rs:176 - ❌ REMOVE Authorization header
    ↓
router.rs:177 - Insert claims in extensions
    ↓
Forward to upstream MCP backend
    - Stdio: ✅ Can access extensions
    - HTTP: ❌ No header, can't access extensions
```

### 2. Find Backend Policies Access

We need to check if `backendAuth.passthrough` is configured. Let's find where policies are accessible:

```bash
cd /Users/xvxy006/Pictures/Git_Repos/agentgateway

# Find where backend policies are defined
grep -rn "BackendAuth\|backend_policies" crates/agentgateway/src/mcp/

# Find the struct definition
grep -rn "struct.*Backend" crates/agentgateway/src/types/
```

### 3. Proposed Code Changes

#### File: `crates/agentgateway/src/mcp/router.rs`

**Current Code (lines 168-178):**
```rust
if let Ok(TypedHeader(Authorization(bearer))) = req
    .extract_parts::<TypedHeader<Authorization<Bearer>>>()
    .await
{
    match auth.jwt_validator.validate_claims(bearer.token()) {
        Ok(claims) => {
            ctx.with_jwt(&claims);
            req.headers_mut().remove(http::header::AUTHORIZATION);  // ❌ Remove this
            req.extensions_mut().insert(claims);
        },
```

**Proposed Fix Option A - Conditional Removal:**
```rust
if let Ok(TypedHeader(Authorization(bearer))) = req
    .extract_parts::<TypedHeader<Authorization<Bearer>>>()
    .await
{
    match auth.jwt_validator.validate_claims(bearer.token()) {
        Ok(claims) => {
            ctx.with_jwt(&claims);
            
            // NEW: Check if backendAuth passthrough is configured
            let should_forward_header = backend
                .map(|b| b.policies.as_ref())
                .and_then(|p| p.backend_auth.as_ref())
                .map(|ba| ba.is_passthrough())
                .unwrap_or(false);
            
            // Only remove header if NOT forwarding
            if !should_forward_header {
                req.headers_mut().remove(http::header::AUTHORIZATION);
            }
            
            req.extensions_mut().insert(claims);
        },
```

**Proposed Fix Option B - Always Keep Header:**
```rust
if let Ok(TypedHeader(Authorization(bearer))) = req
    .extract_parts::<TypedHeader<Authorization<Bearer>>>()
    .await
{
    match auth.jwt_validator.validate_claims(bearer.token()) {
        Ok(claims) => {
            ctx.with_jwt(&claims);
            // REMOVED: req.headers_mut().remove(http::header::AUTHORIZATION);
            req.extensions_mut().insert(claims);
        },
```

Option B is simpler and the header is already validated, so there's no security risk in keeping it.

### 4. Testing the Fix

#### Test 1: Rust HTTP MCP Server
After fix, Rust server should log:
```
✓ JWT Claims received from AgentGateway!
✓ Authorization header present: Bearer eyJhbGc...
```

#### Test 2: Python HTTP MCP Server
After fix, Python server should receive:
```python
{
    'authorization': 'Bearer eyJhbGc...',  # ✅ Header present!
    'content-type': 'application/json',
    ...
}
```

#### Test 3: Existing Tests
Run existing test suite to ensure no regressions:
```bash
cd /Users/xvxy006/Pictures/Git_Repos/agentgateway
cargo test --package agentgateway mcp
```

### 5. Implementation Checklist

- [ ] **Step 1:** Create feature branch
  ```bash
  cd /Users/xvxy006/Pictures/Git_Repos/agentgateway
  git checkout -b fix/mcp-http-auth-forwarding
  ```

- [ ] **Step 2:** Locate the exact line in `router.rs`
  ```bash
  grep -n "req.headers_mut().remove(http::header::AUTHORIZATION)" \
    crates/agentgateway/src/mcp/router.rs
  ```

- [ ] **Step 3:** Implement fix (choose Option A or B)
  - Edit `crates/agentgateway/src/mcp/router.rs`
  - Remove or conditionally remove the header removal line

- [ ] **Step 4:** Build and test
  ```bash
  cargo build
  cargo test --package agentgateway
  ```

- [ ] **Step 5:** Test with real MCP servers
  - Test with Rust MCP server (port 8010)
  - Test with Python MCP server (port 8009)
  - Verify Authorization header is received

- [ ] **Step 6:** Add tests (if needed)
  - Add test case in `crates/agentgateway/src/mcp/mcp_tests.rs`
  - Test that header is forwarded when `backendAuth: passthrough`
  - Test that header is removed when no passthrough configured

- [ ] **Step 7:** Update documentation
  - Update examples showing backendAuth passthrough
  - Document the behavior change

- [ ] **Step 8:** Commit and create PR
  ```bash
  git add crates/agentgateway/src/mcp/router.rs
  git commit -m "fix(mcp): Forward Authorization header to HTTP backends with passthrough"
  git push origin fix/mcp-http-auth-forwarding
  ```

## PR Description Template

```markdown
## What

Fixes HTTP Authorization header forwarding to MCP backends when `backendAuth: passthrough` is configured.

## Why

Currently, the Authorization header is removed after JWT validation (line 176 in `router.rs`) and never restored, preventing HTTP-based MCP servers from authenticating with downstream services.

This affects all HTTP MCP backends regardless of language (Python, Node.js, Go, etc.) - even Rust HTTP backends don't receive the header.

## Changes

- Removed `req.headers_mut().remove(http::header::AUTHORIZATION)` in router.rs line 176
- Header is now forwarded to HTTP backends when validated

## Testing

Tested with:
- ✅ Rust HTTP MCP server - receives Authorization header
- ✅ Python HTTP MCP server - receives Authorization header  
- ✅ Existing test suite passes
- ✅ Protected MCP tools work with authentication

## Breaking Changes

None - this only affects the new `backendAuth: passthrough` feature which wasn't working for HTTP backends.

Closes #[issue-number]
```

## Alternative: More Sophisticated Solution

If you want to only forward when `backendAuth: passthrough` is explicitly configured:

### Investigation Needed

1. Find how to access backend policies in router.rs
2. Check if `BackendAuth` has a `Passthrough` variant
3. Add conditional logic based on configuration

```bash
# Find BackendAuth definition
cd /Users/xvxy006/Pictures/Git_Repos/agentgateway
grep -rn "enum BackendAuth\|struct BackendAuth" crates/

# Find where backend is passed to router
grep -rn "McpRouter::new\|McpRouter::route" crates/
```

## Quick Start Commands

```bash
# 1. Create branch
cd /Users/xvxy006/Pictures/Git_Repos/agentgateway
git checkout -b fix/mcp-http-auth-forwarding

# 2. Edit the file
code crates/agentgateway/src/mcp/router.rs
# Go to line 176 and remove: req.headers_mut().remove(http::header::AUTHORIZATION);

# 3. Build
cargo build

# 4. Test with your MCP servers
# Terminal 1: cargo run -- -f sample-commerce-auth0.yaml
# Terminal 2: cd ../sample-commerce/mcp-server-rust && cargo run
# Terminal 3: cd ../sample-commerce && python3 test_agentgateway_auth.py

# 5. Commit
git diff  # Review changes
git add crates/agentgateway/src/mcp/router.rs
git commit -m "fix(mcp): Forward Authorization header to HTTP backends"

# 6. Push and create PR
git push origin fix/mcp-http-auth-forwarding
```

## Expected Outcome

After this fix:
- ✅ HTTP MCP backends receive Authorization header
- ✅ Rust servers can access Extension<Claims> AND header
- ✅ Python/Node/Go servers can access header
- ✅ MCP servers can authenticate with downstream services
- ✅ User context preserved throughout the chain

## Questions to Consider

1. **Security:** Is there a reason the header was being removed? 
   - Answer: Likely to prevent accidental leakage, but since we're explicitly configuring passthrough, it's intentional

2. **Backward compatibility:** Will this break existing setups?
   - Answer: No, this only affects HTTP MCP backends which weren't working anyway

3. **Configuration:** Should this be opt-in or automatic with passthrough?
   - Answer: Automatic is simpler and matches user expectation of "passthrough"

## Need Help?

If you get stuck on any step:
1. Check the error message carefully
2. Run `cargo check` for compilation errors
3. Look at existing tests in `mcp_tests.rs` for patterns
4. The AgentGateway team can guide on preferred implementation approach

---

**Ready to implement!** Start with the Quick Start Commands and modify line 176 in `router.rs`.
