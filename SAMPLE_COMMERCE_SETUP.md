# Sample Commerce + agentgateway + Auth0 Setup Guide

This guide walks you through setting up agentgateway to protect your sample-commerce MCP server with Auth0 authentication.

## Architecture Overview

```
┌──────────────┐         ┌──────────────┐         ┌──────────────┐         ┌──────────────┐
│              │         │              │         │              │         │              │
│ MCP Client   │────────▶│ agentgateway │────────▶│ Sample       │────────▶│ Backend      │
│ (Inspector)  │  HTTP   │ (Port 3000)  │  HTTP   │ Commerce MCP │  HTTP   │ Services     │
│              │         │              │         │ (Port 8006)  │         │ (Port 8000)  │
└──────────────┘         └──────────────┘         └──────────────┘         └──────────────┘
       │                         │
       │                         │
       │                         │
       └────────────┬────────────┘
                    │
                    ▼
            ┌──────────────┐
            │              │
            │    Auth0     │
            │   (OAuth)    │
            │              │
            └──────────────┘
```

## Prerequisites

1. **Rust and Cargo** installed (✓ You already have this)
2. **Auth0 tenant** configured
3. **Sample Commerce** services ready to run
4. **Rancher Desktop** (for containerized services if needed)

## Step 1: Configure Auth0

### 1.1 Create API in Auth0

1. Go to Auth0 Dashboard → Applications → APIs
2. Click "Create API"
3. Set:
   - **Name**: `Sample Commerce API`
   - **Identifier**: `api://commerce` (must match `AUTH0_AUDIENCE`)
   - **Signing Algorithm**: RS256

### 1.2 Define API Scopes (Optional but Recommended)

In your API settings, add these scopes:
- `openid` - OpenID Connect authentication
- `profile` - User profile information
- `email` - User email
- `offline_access` - Refresh tokens
- `read:products` - Read product information
- `write:cart` - Modify shopping cart
- `write:orders` - Create orders

### 1.3 Create Application for MCP Client

1. Go to Applications → Create Application
2. Choose "Single Page Application" or "Native"
3. Note the **Client ID** and **Domain**
4. In Settings:
   - Add `http://localhost:6274` to Allowed Callback URLs (for MCP Inspector)
   - Add `http://localhost:6274` to Allowed Web Origins
   - Save changes

### 1.4 Note Your Auth0 Configuration

You'll need:
- **Domain**: `your-tenant.auth0.com` (or your custom domain)
- **Client ID**: From the application you created
- **Audience**: `api://commerce` (or your custom API identifier)

## Step 2: Update Configuration Files

### 2.1 Update agentgateway Configuration

Edit `/Users/xvxy006/Pictures/Git_Repos/agentgateway/sample-commerce-auth0.yaml`:

```yaml
# Replace these three lines with your actual Auth0 values:
issuer: https://YOUR_TENANT.auth0.com/
audiences:
- api://commerce
jwks:
  url: https://YOUR_TENANT.auth0.com/.well-known/jwks.json
```

**Example with real values:**
```yaml
issuer: https://dev-abc123.us.auth0.com/
audiences:
- api://commerce
jwks:
  url: https://dev-abc123.us.auth0.com/.well-known/jwks.json
```

### 2.2 Update Sample Commerce .env

Edit `/Users/xvxy006/Pictures/Git_Repos/sample-commerce/.env`:

```bash
# Update these values to match your Auth0 tenant
AUTH0_DOMAIN=your-tenant.auth0.com
AUTH0_AUDIENCE=api://commerce
AUTH0_ISSUER=https://your-tenant.auth0.com/
AUTH0_CLIENT_ID=your-device-flow-client-id
```

## Step 3: Build agentgateway

```bash
cd /Users/xvxy006/Pictures/Git_Repos/agentgateway

# Build the project (first time will take a while)
cargo build --release

# Or just run directly (builds automatically)
cargo run --release -- --help
```

## Step 4: Start All Services

Open **4 terminal windows** or tabs:

### Terminal 1: Backend Services
```bash
cd /Users/xvxy006/Pictures/Git_Repos/sample-commerce
docker compose up
# Or if using Rancher Desktop
./start-dev.sh
```

Wait until all services are healthy.

### Terminal 2: Sample Commerce MCP Server
```bash
cd /Users/xvxy006/Pictures/Git_Repos/sample-commerce/mcp-server
source .venv/bin/activate
python mcp_server.py
```

You should see:
```
Server running on http://localhost:8006/mcp
```

### Terminal 3: agentgateway
```bash
cd /Users/xvxy006/Pictures/Git_Repos/agentgateway
cargo run --release -- -f sample-commerce-auth0.yaml
```

You should see:
```
Starting agentgateway...
Listening on 0.0.0.0:3000
```

### Terminal 4: Test Commands (Optional)
Keep this terminal free for testing commands.

## Step 5: Test the Setup

### 5.1 Test Without Authentication (Should Fail)

```bash
curl -i http://localhost:3000/commerce/mcp
```

**Expected Response:**
```
HTTP/1.1 401 Unauthorized
WWW-Authenticate: Bearer resource_metadata="http://localhost:3000/.well-known/oauth-protected-resource/commerce/mcp"
...
```

This is correct! The gateway is protecting your MCP server.

### 5.2 Check Resource Metadata

```bash
curl -s http://localhost:3000/.well-known/oauth-protected-resource/commerce/mcp | jq
```

**Expected Response:**
```json
{
  "resource": "http://localhost:3000/commerce/mcp",
  "authorization_servers": [
    "http://localhost:3000/.well-known/oauth-authorization-server/commerce/mcp"
  ],
  "scopes_supported": [
    "openid",
    "profile",
    "email",
    "offline_access",
    "read:products",
    "write:cart",
    "write:orders"
  ],
  "bearer_methods_supported": [
    "header",
    "body",
    "query"
  ],
  "resource_documentation": "http://localhost:3000/commerce/docs",
  "resource_policy_uri": "http://localhost:3000/commerce/policies"
}
```

### 5.3 Test with MCP Inspector

1. **Install MCP Inspector** (if not already installed):
   ```bash
   npm install -g @modelcontextprotocol/inspector
   ```

2. **Start MCP Inspector**:
   ```bash
   npx @modelcontextprotocol/inspector
   ```

3. **Open in Browser**: http://localhost:6274

4. **Configure Connection**:
   - Transport: **Streamable HTTP**
   - URL: `http://localhost:3000/commerce/mcp`
   - Click "Connect"

5. **Authenticate**:
   - Inspector will detect the 401 response
   - Follow the OAuth flow
   - Login with your Auth0 credentials
   - You'll be redirected back to Inspector

6. **Test Tools**:
   - Try `list_products` (should work - public)
   - Try `login_user` (should trigger Auth0 login)
   - Try `get_cart` (should work after authentication)

## Step 6: Monitor and Observe

### 6.1 View Metrics

```bash
# Check agentgateway metrics
curl http://localhost:15020/metrics
```

### 6.2 Enable Tracing (Optional)

To enable distributed tracing:

1. **Start Jaeger** (with Docker/Rancher Desktop):
   ```bash
   docker run -d --name jaeger \
     -p 16686:16686 \
     -p 4317:4317 \
     jaegertracing/all-in-one:latest
   ```

2. **Uncomment tracing in config**:
   Edit `sample-commerce-auth0.yaml`, uncomment:
   ```yaml
   config:
     tracing:
       otlpEndpoint: http://localhost:4317
   ```

3. **Restart agentgateway**

4. **View traces**: http://localhost:16686

## Troubleshooting

### Issue: agentgateway won't start

**Error**: `error: no field named 'jwksUrl'`

**Solution**: Update to latest syntax:
```yaml
jwks:
  url: https://your-tenant.auth0.com/.well-known/jwks.json
```

### Issue: 401 Unauthorized even with token

**Check**:
1. Token audience matches configuration
2. Token is not expired
3. Auth0 domain in config matches token issuer

**Debug**:
```bash
# Decode your JWT token
echo "YOUR_TOKEN" | cut -d. -f2 | base64 -d | jq
```

### Issue: Sample Commerce MCP server not reachable

**Check**:
```bash
# Test direct connection
curl http://localhost:8006/mcp
```

If this fails, check:
1. MCP server is running
2. Port 8006 is not in use by another process
3. Check MCP server logs for errors

### Issue: CORS errors in browser

**Solution**: Already configured in the YAML:
```yaml
cors:
  allowOrigins:
  - '*'
```

For production, restrict to specific origins:
```yaml
cors:
  allowOrigins:
  - 'http://localhost:6274'
  - 'https://your-app.com'
```

## Next Steps

### Add Authorization Policies

Beyond authentication, add fine-grained authorization:

```yaml
policies:
  authorization:
    rules:
    - name: require-cart-scope
      match:
        path:
          prefix: /commerce/mcp
      cel: 'has(request.auth.claims.scope) && request.auth.claims.scope.contains("write:cart")'
```

### Add Rate Limiting

Protect against abuse:

```yaml
policies:
  rateLimit:
    tokensPerSecond: 10
    burstSize: 20
```

### Add Prompt Guards

Filter sensitive content:

```yaml
policies:
  ai:
    promptGuard:
      request:
        regex:
          action:
            reject:
              response:
                body: "Request blocked"
          rules:
          - builtin: ssn
          - builtin: credit_card
```

### Monitor in Production

1. Set up Prometheus scraping: `http://localhost:15020/metrics`
2. Configure alerting rules
3. Set up log aggregation
4. Monitor error rates and latency

## Architecture Benefits

With agentgateway in front of your sample-commerce MCP server, you get:

1. ✅ **Centralized Authentication**: No auth code in your MCP server
2. ✅ **Token Validation**: Automatic JWT validation
3. ✅ **OAuth Flow Handling**: Auth0 adapter handles protocol quirks
4. ✅ **Observability**: Built-in metrics and tracing
5. ✅ **Security**: Rate limiting, prompt guards, CORS
6. ✅ **Scalability**: High-performance Rust proxy
7. ✅ **Flexibility**: Easy to add policies without changing code

## Comparison with Your Current Setup

### Before (with mcp-auth-service):
```
MCP Client → MCP Server → mcp-auth-service → Auth0
```

### After (with agentgateway):
```
MCP Client → agentgateway → MCP Server
                ↓
              Auth0
```

### Key Differences:

| Feature | mcp-auth-service | agentgateway |
|---------|------------------|--------------|
| Language | Python | Rust |
| Performance | Moderate | High |
| Auth in MCP | Yes (via client) | No (handled by gateway) |
| Token Validation | In MCP server | In gateway |
| Other Features | Auth only | Auth + Observability + Policies |
| Protocol Support | MCP | MCP + A2A + HTTP |
| Maintenance | Custom code | Open source project |

## Support

- **agentgateway docs**: https://agentgateway.dev/docs
- **agentgateway Discord**: https://discord.gg/BdJpzaPjHv
- **Auth0 docs**: https://auth0.com/docs
- **MCP spec**: https://modelcontextprotocol.io

Enjoy your secure MCP server! 🚀
