## Rate Limiting Example

This example shows how to use the agentgateway to perform rate limiting, MCP native authorization, and multiplex with telemetry.

It is recommended to complete the [basic](../basic), [authorization](../authorization) and [multiplex](../multiplex) examples before this one.

### Running the example

```bash
cargo run -- -f examples/ratelimiting/config.yaml
```

In addition to the basic configuration from the [basic](../basic) [authorization](../authorization) and [multiplex](../multiplex) examples, we have a few new fields:

The `localRateLimit` indicates how to configure local rate limiting.

```yaml
    - policies:
        localRateLimit:
          - maxTokens: 10
            tokensPerFill: 1
            fillInterval: 60s
```

To adjust rate limiting configuration, you may increase the `maxTokens` in the `config.yaml` as needed.

To test the authorization, users will be required to pass a valid JWT token matching the criteria, refer to the [authorization](../authorization) for details.

To test the multiplex, comment out the following lines:
```
          # - name: time
          #   stdio:
          #     cmd: uvx
          #     args: ["mcp-server-time"]%
```

To configure the `test-user`'s access to the tool from the `mcp-server-time`, uncomment the following line:

```
          # - 'jwt.sub == "test-user" && mcp.tool.name == "get_current_time"'
```

Refer to the [telemetry](../telemetry) example to learn how to visualize metrics and tracing provided by agentgateway for your MCP servers.
