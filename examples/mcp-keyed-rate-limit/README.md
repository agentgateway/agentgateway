## MCP keyed local rate limiting

Give each authenticated client a separate allowance for each tool. A busy client
can exhaust one tool's allowance while other tools and other clients remain usable.
This example uses existing local rate limiting; it needs no external rate limit service.

The API-key metadata assigns two credentials to `alice` and one to `bob`. The
bucket key uses that configured identity, not the credential or MCP session ID,
so changing Alice's key or opening another session does not grant a new allowance.
The key includes `mcp.tool.target` as well as `mcp.tool.name`, keeping identically
named tools on different targets in separate buckets. `prefixMode: always` exposes
public names such as `everything_echo`; policy context identifies the target and
tool separately.

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

The condition limits tool calls only. Initialization, tool discovery and session
cleanup do not spend the tool allowance and remain available after it is exhausted.
They still require authentication. Add a separate traffic limit if control requests
also need protection.

### Try it

Use agentgateway v1.6.0 or later and Node.js 24 or later for the Everything test
server. From the repository root, start the gateway:

```bash
cargo run -- -f examples/mcp-keyed-rate-limit/config.yaml
```

Alternatively, use an installed `agentgateway` binary with the same `-f` argument.
In [MCP Inspector](https://github.com/modelcontextprotocol/inspector), connect using
Streamable HTTP to `http://localhost:3000/mcp` and set the `Authorization` header
to `Bearer alice-key`. The example keys are public test credentials; replace them
before using this configuration outside a local demonstration.

Starting with fresh buckets, perform these steps before the 60-second refill:

| Client credential | Action | Expected result |
| --- | --- | --- |
| `alice-key` | Call `everything_echo` twice with `{"message":"hello"}` | Both succeed |
| `alice-key` | Call `everything_echo` a third time | Rate-limit tool error |
| `alice-key` | List tools | Discovery still works |
| `alice-key` | Call `everything_get-sum` with `{"a":1,"b":2}` | Succeeds |
| `alice-rotated-key`, in a new connection | Call `everything_echo` | Still rate-limited |
| `bob-key`, in a new connection | Call `everything_echo` | Succeeds |

A denied tool call returns an MCP result with `isError: true` and retry guidance.
Do not interpret HTTP success alone as tool success. Failed authentication is a
separate error. Disconnect the Inspector sessions when finished.

### Scope of the allowance

`maxTokens` is the initial and maximum burst allowance. Every `fillInterval`,
`tokensPerFill` replenishes the bucket up to that maximum. This is a token bucket,
not a fixed calendar-minute quota.

Use stable authenticated identity metadata, such as `apiKey.client`, `jwt.sub`, or
an identity returned by trusted external authorization. Avoid caller-controlled
headers, request IDs, session IDs and tool arguments. If identity values can
contain `/`, choose an unambiguous encoding for the compound key. A missing, empty
or unevaluable key uses a shared fallback bucket; it does not exempt a request.

Buckets are in memory and local to each gateway process. Restarting the gateway
resets them; multiple replicas each have their own allowance. Keyed storage is
bounded, and an evicted key receives a fresh bucket when used again. These limits
shape traffic, rather than providing durable billing quotas. Use
[`remoteRateLimit`](../traffic-ratelimiting-global/README.md) for a shared fleet-wide
quota. When several rate-limit rules apply, every rule must admit the request.
