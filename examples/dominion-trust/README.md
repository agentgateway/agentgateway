# Dominion Observatory Trust Verification

This example demonstrates how to use agentgateway's `extAuthz` policy to verify MCP server behavioral trust scores via [Dominion Observatory](https://dominion-observatory.sgdata.workers.dev) before allowing tool calls.

## How it works

1. An agent sends an MCP tool call through agentgateway
2. Agentgateway's `extAuthz` policy sends a check request to the trust verification sidecar
3. The sidecar queries the Dominion Observatory API: `GET /benchmark/{server_name}`
4. If the server's trust score is below the threshold (default: 60), the request is denied with a 403
5. If the server is trusted, the request proceeds to the MCP backend

Trust scores are cached for 5 minutes to avoid excessive API calls.

## Setup

### 1. Start the trust verification sidecar

```bash
python trust-authz-server.py --port 8990 --threshold 60
```

Options:
- `--port` - Port for the sidecar (default: 8990)
- `--threshold` - Minimum trust score to allow requests (0-100, default: 60)
- `--cache-ttl` - Cache duration in seconds (default: 300)

### 2. Start your MCP server

Start your MCP server on `localhost:3001` (or adjust `config.yaml` accordingly).

### 3. Start agentgateway

```bash
agentgateway --config examples/dominion-trust/config.yaml
```

## API Reference

Dominion Observatory provides behavioral trust scoring for 14,820+ MCP servers:

```
GET https://dominion-observatory.sgdata.workers.dev/benchmark/{server_name}

Response: {"trust_score": 0-100, ...}
```

## Configuration

Edit `config.yaml` to adjust:
- MCP backend targets and ports
- `failureMode`: `Deny` (default) blocks requests when the sidecar is unavailable; `Allow` lets them through
