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

This follows the SEP-993 namespacing direction discussed in [modelcontextprotocol/modelcontextprotocol#94](https://github.com/modelcontextprotocol/modelcontextprotocol/discussions/94).

### Resource Handling

Resource URIs are automatically wrapped in an `agw://` scheme (e.g., `agw://everything/?u=memo%3A%2F%2Finsights`). This allows the gateway to route requests back to the correct origin server while preserving URI templates for AI clients.

### Full Manual E2E (Diverse Servers)

For a realistic end-to-end matrix (mixed transports + mixed capability surfaces), start local HTTP/SSE upstreams in one terminal:

```bash
PORT=4101 npx -y @modelcontextprotocol/server-everything streamableHttp &
PORT=4201 npx -y @modelcontextprotocol/server-everything sse &
```

In a second terminal:

```bash
RUST_MIN_STACK=8388608 cargo run -- -f examples/multiplex/feature-test.yaml
```

Then connect MCP Inspector to:

`http://localhost:3001/mcp`

Notes:
- The gateway config also starts stdio targets for:
  - `time` (`uvx mcp-server-time`)
  - `fetch` (`uvx mcp-server-fetch`)
  - `git` (`uvx mcp-server-git --repository .`)
  - `filesystem` (`npx -y @modelcontextprotocol/server-filesystem .`)
- The config includes one intentionally broken upstream (`127.0.0.1:5999`) to validate fail-open behavior.
- Make sure `uvx` and `npx` are available in your shell.
