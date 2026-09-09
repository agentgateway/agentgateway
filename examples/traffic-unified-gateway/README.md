## Unified Gateway Example

This example exposes LLM, MCP, and the agentgateway UI on one gateway listener.
The UI is protected with the same local Keycloak OIDC setup used by the
`traffic-oidc` example.

The `default` gateway listens on port 3000. Because the gateway is named
`default`, the top-level `llm`, `mcp`, and `ui` sections attach to it without
setting explicit `gateways` fields.

### Choosing a listen address

Set `bindAddress` on a gateway to choose its IPv4 or IPv6 listen address:

```yaml
gateways:
  default:
    port: 3000
    bindAddress: 127.0.0.1
  shared:
    port: 4000
    bindAddress: 0.0.0.0
```

Here, the `default` gateway accepts connections only through IPv4 loopback.
In a Kubernetes pod without `hostNetwork`, all containers in that pod can
connect to it, but other pods cannot connect directly. Use `::1` for IPv6 loopback.
The `shared` gateway listens on all IPv4 interfaces; attach routes to it using
`gateways: shared`.

When `bindAddress` is omitted, the existing default remains: `::` on Unix with
IPv6 enabled, otherwise `0.0.0.0`. All named listeners within a gateway share
its address and port. Gateways must still use different ports, even when their
addresses differ.

### Running the example

Set the provider API key:

```bash
export OPENAI_API_KEY=...
```

Start the local OIDC issuer:

```bash
docker compose -f examples/traffic-oidc/docker-compose.yaml up -d
```

Export the browser-auth cookie secret:

```bash
export OIDC_COOKIE_SECRET="$(python3 -c 'import os; print(os.urandom(32).hex())')"
```

Start agentgateway:

```bash
cargo run -- -f examples/traffic-unified-gateway/config.yaml
```

### Accessing the UI

The UI is configured to be OIDC-protected, allowing authorized users with the `@example.com` domain.
The Keycloak OIDC provider is configured to use the `testuser` and `testpass` credentials, and is registered to `test@example.com`.

Access the UI at `http://localhost:3000/` and login.

### Sending LLM requests

The LLM request path is secured with API Key authentication.

Send an LLM request through the shared gateway:

```bash
curl http://localhost:3000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer agw_sk_example" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {
        "role": "user",
        "content": "Say hello from the unified gateway"
      }
    ]
  }'
```

### Sending MCP requests

The MCP endpoint is available at `http://localhost:3000/mcp`.
This example does not add any authentication, but authentication can be added: see [MCP Authentication](../mcp-authentication).

### Teardown

Stop the local OIDC issuer with:

```bash
docker compose -f examples/traffic-oidc/docker-compose.yaml down
```
