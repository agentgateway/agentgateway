# opencode → agentgateway → Anthropic

This example shows how to route opencode's Anthropic traffic through
agentgateway so the gateway holds the API key and opencode never contacts
Anthropic directly.

```
opencode  →  agentgateway (localhost:3000)  →  api.anthropic.com
```

Benefits:
- The Anthropic API key lives only in the gateway (or its environment), not
  in the opencode config that may be committed to source control.
- You can layer agentgateway policies on top: prompt guards, rate limiting,
  cost tracking, access logging, etc.

## Prerequisites

- agentgateway built (`cargo build --release` from the repo root)
- opencode installed (`npm i -g opencode-ai` or from source)
- An Anthropic API key

## Ports

agentgateway listens on two ports:

| Port | Purpose |
|------|---------|
| **3000** | Proxy — receives requests from opencode and forwards to Anthropic |
| **15000** | Admin — usage stats, config dump, debug endpoints |

## 1. Start agentgateway

Export your Anthropic API key and start the gateway from the repo root:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
./target/release/agentgateway --file examples/opencode-anthropic/gateway.yaml
```

The gateway binds to `http://localhost:3000` and accepts Anthropic Messages
API requests at `/v1/messages`.

## 2. Configure opencode

Copy the provided `opencode.json` to your opencode config directory, or merge
the `provider.anthropic` block into your existing config:

```bash
cp examples/opencode-anthropic/opencode.json ~/.config/opencode/opencode.json
```

The key change is setting `baseURL` so the Anthropic SDK inside opencode sends
requests to the gateway instead of `api.anthropic.com`.  The `apiKey` value is
a placeholder — the real key is held by the gateway:

```json
{
  "provider": {
    "anthropic": {
      "options": {
        "baseURL": "http://localhost:3000/v1",
        "apiKey": "placeholder"
      }
    }
  }
}
```

## 3. Run opencode

```bash
opencode
```

Pick any `claude-*` model.  Traffic flows:

```
opencode (Anthropic SDK)
  POST http://localhost:3000/v1/messages
    → agentgateway
      adds x-api-key: <real key>
        → POST https://api.anthropic.com/v1/messages
```

## Verifying the gateway is in the path

With the gateway running, confirm it is intercepting requests:

```bash
curl -s http://localhost:3000/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-6",
    "max_tokens": 32,
    "messages": [{"role": "user", "content": "Say hello"}]
  }' | jq .
```

You should get a valid Anthropic response back through the gateway.

## Viewing token usage and cost

After sending some requests through the gateway, query the **admin server**
at port **15000** (not the proxy port 3000):

```bash
curl -s http://localhost:15000/usage | jq .
```

Example output after a few opencode sessions:

```json
[
  {
    "userId": "anonymous",
    "model": "claude-sonnet-4-6",
    "inputTokens": 23,
    "outputTokens": 29,
    "cacheReadTokens": 0,
    "cacheWriteTokens": 0,
    "costUsd": 0.000504,
    "requestCount": 2,
    "firstSeen": "2026-04-29T10:08:39Z",
    "lastSeen": "2026-04-29T10:08:41Z"
  }
]
```

The `costUsd` is computed from the pricing table in `gateway.yaml`.
Current Anthropic prices are pre-filled; update them at
https://www.anthropic.com/pricing if they change.

Filter by user or model with query parameters:

```bash
# All requests for a specific model
curl -s "http://localhost:15000/usage?model=claude-sonnet-4-6" | jq .

# Requests since a Unix timestamp
curl -s "http://localhost:15000/usage?since=1748000000" | jq .
```

When JWT or API key authentication is added to the gateway listener, the
`userId` field will be the authenticated identity instead of `"anonymous"`.

To persist usage across gateway restarts, add to `gateway.yaml`:

```yaml
config:
  usageStorePath: /var/lib/agentgateway/usage.json
  pricing:
    ...
```

## Adding policies

Edit `gateway.yaml` to add policies before the backend.  For example, to
reject any prompt containing a credit-card number:

```yaml
      policies:
        ai:
          promptGuard:
            request:
            - regex:
                action: reject
                rules:
                - builtin: credit_card
```

See the [agentgateway documentation](https://agentgateway.dev) for the full
policy reference.
