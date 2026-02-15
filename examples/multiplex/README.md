## Multiplex Example

In the [basic](../basic) example, we exposed a single MCP server.
Agentgateway can also multiplex multiple MCP servers, and expose them as a single MCP server to clients.

This centralizes and simplifies client configuration -- as we add and remove tools, only the gateway configuration needs to change, rather than all MCP clients.

### Running the example

```bash
cargo run -- -f examples/multiplex/config.yaml
```

Multiplexing is enabled by adding multiple `targets` to an MCP backend. Here we serve the `everything` and `time` servers.

```yaml
targets:
- name: time
  stdio:
    cmd: uvx
    args: ["mcp-server-time"]
- name: everything
  stdio:
    cmd: npx
    args: ["@modelcontextprotocol/server-everything"]
```

When you connect an MCP client, you will see tools from both servers. 

### Namespacing (SEP-993)

To avoid collisions, identifiers are prefixed with the target name followed by `__` (double underscore). For example, the `echo` tool from the `everything` server becomes `everything__echo`.

### Resource Handling

Resource URIs are automatically wrapped in an `agw://` scheme (e.g., `agw://everything/?u=memo%3A%2F%2Finsights`). This allows the gateway to route requests back to the correct origin server while preserving URI templates for AI clients.
