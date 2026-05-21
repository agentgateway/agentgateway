## MCP Backend with CORS Example

This example shows how to expose an MCP backend through agentgateway with a CORS policy tuned for browser-based MCP clients. It builds on the [basic](../basic/README.md) MCP-over-stdio example by adding the CORS headers required for streamable HTTP MCP clients running in a browser.

### Running the example

```bash
cargo run -- -f examples/mcp-cors/config.yaml
```

The gateway listens on port `8081` and spawns the `@modelcontextprotocol/server-everything` MCP server over stdio on demand.

You can connect with the [MCP inspector](https://github.com/modelcontextprotocol/inspector):

```bash
npx @modelcontextprotocol/inspector
```

Point it at `http://localhost:3000/mcp` (streamable HTTP) or `http://localhost:3000/sse`.

### The CORS policy

MCP's streamable HTTP transport relies on a few non-standard headers that must be explicitly allowed and exposed for browser clients:

```yaml
policies:
  cors:
    allowOrigins: ["*"]
    allowHeaders:
    - mcp-protocol-version
    - content-type
    - cache-control
    exposeHeaders:
    - Mcp-Session-Id
```

* `mcp-protocol-version` — sent by clients to negotiate the MCP protocol version.
* `Mcp-Session-Id` — returned by the server and must be `exposeHeaders` so browser code can read it and include it on subsequent requests.

Restrict `allowOrigins` to your client's origin for production deployments.
