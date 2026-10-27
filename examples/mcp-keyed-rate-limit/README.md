## MCP keyed local rate limiting

This example gives each API key client its own allowance for each tool on each MCP target, so
one busy client cannot use up the allowance for everyone else.

The `localRateLimit` rule in [config.yaml](config.yaml) only counts `tools/call` requests, so clients
can still initialize and list tools after reaching a limit. Its `key` combines the client from the
API key metadata with the tool's target and name, and each distinct value gets its own bucket. Two
credentials with the same `client` metadata share an allowance, and reconnecting does not reset
it. A call whose key cannot be evaluated, such as one to an unknown tool, uses a single bucket
shared by all clients.

Buckets live in the proxy instance that created them, so they reset on restart and each replica
holds the configured limit. Use `remoteRateLimit` for a quota shared across a fleet.

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
