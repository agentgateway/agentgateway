## MCP keyed local rate limiting

I use my MCP gateway across multiple clients and servers, and I don't want one
client to use up the allowance for everyone. This setup allows each client its
own allowance for each tool on each server. Two credentials belonging to the
same client still share the allowance.

When a client reaches its limit, its calls to that particular tool are temporarily
rejected. This does not affect other tools or other clients' allowances. Changing
credentials or reconnecting does not provide a fresh allowance. The client can
still list the available tools.

```yaml
localRateLimit:
  conditional:
  - condition: 'mcp.methodName == "tools/call"'
    type: requests
    maxTokens: 2
    tokensPerFill: 2
    fillInterval: 60s
    key: 'apiKey.client + "/" + mcp.tool.target + "/" + mcp.tool.name'
```

In this example, it starts with an allowance of two calls per client, server, and
tool. Every 60 seconds, it adds two calls back, up to a maximum of two. The counters
live inside one gateway process. If the process is restarted, the counters reset.
Gateway replicas have separate counters, and a shared limit requires an external
rate-limit service, which this example does not use.

### Try it

You can try it by starting the gateway with the example configuration:

```bash
cargo run -- -f examples/mcp-keyed-rate-limit/config.yaml
```

Then connect using [MCP Inspector](https://github.com/modelcontextprotocol/inspector)
and the supplied test credentials:

```text
Transport: Streamable HTTP
URL: http://localhost:3000/mcp
Authorization: Bearer alice-key
```

Use Alice's echo tool twice; the third call should be rejected. Alice's other tool
and Bob's echo call should still work. Connecting with Alice's second key should
still be limited.

| Credential | Tool | Arguments |
| --- | --- | --- |
| `alice-key` | `everything_echo` | `{"message":"hello"}` |
| `alice-key` | `everything_get-sum` | `{"a":1,"b":2}` |
| `alice-rotated-key` | `everything_echo` | `{"message":"hello"}` |
| `bob-key` | `everything_echo` | `{"message":"hello"}` |

These are public demonstration keys, so they must be replaced before using this
configuration outside a local test.
