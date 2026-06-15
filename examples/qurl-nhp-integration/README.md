# qURL + OpenNHP Integration Example

This example demonstrates how to use **qURL** (secure time-limited access links) with **OpenNHP** (Network Hiding Protocol) in agentgateway to provide **just-in-time access** to AI models and MCP servers.

## Architecture

```
┌─────────────┐     ┌──────────────────┐     ┌─────────────────┐     ┌──────────────────┐
│ AI Client   │────▶│  agentgateway    │────▶│  qURL API       │────▶│  OpenNHP Server  │
│ (Chat, API) │     │  (qurlNHP        │     │  (layerv.ai)    │     │  + NHP-AC        │
└─────────────┘     │   provider)      │     └────────┬────────┘     └────────┬─────────┘
                    │                  │              │                      │
                    │  1. Resolve      │              │                      │
                    │  qURL token      │              │                      │
                    │◀─────────────────│              │                      │
                    │  2. Returns      │              │   3. Triggers        │
                    │  target_url +    │              │   NHP knock          │
                    │  access_grant    │              │   (opens firewall)   │
                    │                  │              │◀─────────────────────│
                    │  4. Request to   │              │                      │
                    │  target_url      │─────────────▶│                      │
                    │  (now accessible)│              │                      │
                    └──────────────────┘              └──────────────────────┘
```

## Quick Start

### 1. Prerequisites

- qURL account at [layerv.ai](https://layerv.ai)
- qURL API key with `qurl:resolve` scope
- qURL Resource (`r_*`) created for your target (model endpoint, MCP server, etc.)
- OpenNHP infrastructure protecting the target (NHP-Server, NHP-AC)

### 2. Configure

1. Copy `config.yaml` and set your environment variables:
   ```bash
   export QURL_API_KEY="qurl_sk_your_api_key_here"
   export OTEL_EXPORTER_OTLP_ENDPOINT="http://localhost:4318"
   ```

2. Update resource IDs in config.yaml to match your qURL resources

### 3. Run

```bash
# From agentgateway root
cargo run --bin agentgateway -- --config examples/qurl-nhp-integration/config.yaml
```

### 4. Test

```bash
# Test LLM chat completion
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-hidden",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'

# Test MCP tools
curl -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

## How It Works

### qURL Resolution Flow

1. **Request arrives** at agentgateway for `gpt-4o-hidden` model
2. **qurlNHP provider** intercepts the request
3. **Calls qURL API** `POST /v1/resolve` with the access token
4. **qURL API** validates token, triggers **NHP knock** to OpenNHP Server
5. **NHP Server** validates, queries ASP, instructs NHP-AC to allow client IP
6. **qURL API** returns `target_url` + `access_grant` (expires_in, src_ip)
7. **agentgateway** forwards request to resolved `target_url`
8. **Network access auto-expires** after `session_duration`

### Configuration Options

| Option | Description | Default |
|--------|-------------|---------|
| `model` | Model name to send to provider | From request |
| `api_url` | qURL API base URL | `https://api.layerv.ai` |
| `api_key` | API key with `qurl:resolve` scope | **Required** |
| `resource_id` | qURL Resource ID (`r_*`) | **Required** (or token) |
| `token` | Direct qURL access token (`at_*`) | Alternative to resource_id |
| `nhp_agent_id` | NHP Agent ID for bootstrap | Optional |
| `formats` | Supported API formats | `[{type: "qurlNHP"}]` |
| `cache_ttl` | Cache resolved URL | From `access_grant.expires_in` |

### Supported Formats

The `qurlNHP` provider format uses OpenAI-compatible API paths:
- `completions` → `/v1/chat/completions`
- `messages` → `/v1/messages` (Anthropic)
- `responses` → `/v1/responses` (OpenAI Responses API)
- `embeddings` → `/v1/embeddings`

## Advanced Usage

### Per-Request Tokens

For dynamic access (e.g., user provides token in request):

```yaml
policies:
  backendAuth:
    qurl:
      api_key: ${QURL_API_KEY}
      token_expression: 'request.headers["x-qurl-token"]'
backends:
  - ai:
      name: dynamic-model
      provider:
        qurlNHP:
          model: gpt-4o
          api_url: https://api.layerv.ai
          api_key: ${QURL_API_KEY}
          # token comes from request header at runtime
```

### MCP Server Protection

Protect MCP servers behind qURL:

```yaml
mcp:
  targets:
    - name: secure-tools
      host: "qurl://r_mcp_tools_789"
      transport: streamableHttp
      auth:
        qurl:
          api_key: ${QURL_API_KEY}
          resource_id: "r_mcp_tools_789"
```

### A2A Agent Discovery

Hide A2A agent cards behind qURL:

```yaml
a2a:
  agents:
    - name: hidden-analyst
      card_url: "qurl://r_agent_card_456/.well-known/agent-card.json"
      auth:
        qurl:
          api_key: ${QURL_API_KEY}
          resource_id: "r_agent_card_456"
```

## Observability

The integration exports these metrics:

- `qurl_resolve_total{status="success|error"}` - Resolution attempts
- `qurl_resolve_duration_seconds` - Latency of qURL API calls
- `qurl_cache_hits_total / qurl_cache_misses_total` - Cache effectiveness
- `nhp_knock_total{status="success|failed|timeout"}` - NHP knock results (via qURL API)

Structured logs include:
- `qurl_token_id` - The token used
- `qurl_resource_id` - The resource ID
- `resolved_target_url` - The URL after resolution
- `access_grant_expires_in` - Seconds until network access expires
- `granted_src_ip` - IP granted access by NHP-AC

## Security Considerations

1. **API Keys**: Store `QURL_API_KEY` in secret management, not config
2. **Token Scope**: Use API keys with minimal `qurl:resolve` scope only
3. **TLS**: qURL API calls use HTTPS; verify certificates
4. **Audit**: All resolutions logged with request context
5. **Expiry**: Network access auto-revokes via NHP-AC after session_duration

## Troubleshooting

| Issue | Resolution |
|-------|------------|
| `qURL token expired` | Generate new token or use resource with longer `expires_in` |
| `NHP knock failed` | Check OpenNHP Server logs, verify agent registration |
| `No route to target` | Network access may have expired; check `access_grant.expires_in` |
| `Rate limited` | Respect `Retry-After` header, implement backoff |

## References

- [qURL API Documentation](https://layerv.ai/docs)
- [OpenNHP Documentation](https://docs.opennhp.org/)
- [agentgateway Configuration Schema](https://agentgateway.dev/schema/config)