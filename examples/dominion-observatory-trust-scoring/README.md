# AgentGateway + Dominion Observatory Trust Scoring

External authorization example that verifies MCP server trust scores before routing tool calls through AgentGateway.

## How It Works

1. Agent sends MCP tool call to AgentGateway
2. AgentGateway calls the Observatory authz service (ext-authz)
3. Authz service checks the target server's trust score via Observatory API
4. If score >= threshold: request is routed to the upstream MCP server
5. If score < threshold: request is denied with 403

## Quick Start

```bash
docker-compose up
```

Or run the authz service standalone:

```bash
pip install flask httpx
TRUST_THRESHOLD=70 python authz-service.py
```

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `TRUST_THRESHOLD` | 60 | Minimum trust score (0-100) |
| `OBSERVATORY_URL` | https://dominionobservatory.com | Observatory API base URL |
| `CACHE_TTL` | 300 | Trust score cache TTL in seconds |
| `PORT` | 8080 | Authz service port |

## Links

- [Dominion Observatory](https://dominionobservatory.com) - Behavioral trust scoring for 14,800+ MCP servers
- [AgentGateway ext-authz docs](https://agentgateway.dev/docs/policies/ext-authz)
- [Observatory API](https://dominionobservatory.com/api/trust?url=example)
