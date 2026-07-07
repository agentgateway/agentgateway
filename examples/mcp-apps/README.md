## MCP Apps Example

[MCP Apps](https://modelcontextprotocol.io/extensions/apps/overview) (the `io.modelcontextprotocol/ui` extension) lets an MCP server attach an interactive UI to a tool: the server declares a `ui://` resource in the tool's `_meta.ui.resourceUri`, and the host fetches it with `resources/read` and renders it in a sandboxed iframe.

Agentgateway proxies Apps metadata and UI resources:

* Tool `_meta.ui` is preserved, and its `resourceUri` rewritten when multiplexing.
* `ui://` URIs keep their scheme, with the target carried in the URI authority (`ui://<target>+<rest>`), so hosts still see valid Apps resources.
* `_meta.ui.visibility` passes through untouched: per the Apps spec, hiding app-only tools from the model and rejecting app calls to non-app tools are the host's responsibility.
* UI metadata is only advertised for resources the client is authorized to read.

This example uses the reference host and Apps-enabled sample servers from the [ext-apps](https://github.com/modelcontextprotocol/ext-apps) repository. The servers are published to npm, so the gateway launches them with `npx`; the host is not published, so clone and run it from the repository.

### Single server

Start the gateway with the system-monitor sample server:

```bash
cargo run -- -f examples/mcp-apps/config.yaml
```

The config also enables CORS, since the host runs in a browser.

Next, run the reference host, pointing it at the gateway. The repository is an npm workspace; install from its root so the workspace `prepare` build sees all of its dependencies.

```bash
git clone https://github.com/modelcontextprotocol/ext-apps
cd ext-apps
npm install
cd examples/basic-host
SERVERS='["http://localhost:3000/mcp"]' npm run start
```

Open http://localhost:8080 and call the `get-system-info` tool. The host reads the tool's `ui://` resource through the gateway and renders the dashboard, and the dashboard's live updates work: the app inside the iframe calls the `poll-system-stats` tool through the host, and with a single target the gateway passes tool names through unchanged.

`poll-system-stats` is declared with `_meta.ui.visibility: ["app"]`. The gateway forwards it, visibility metadata intact, and the host is responsible for keeping it out of the model's tool list while allowing the app to call it.

### Multiplexing

```bash
cargo run -- -f examples/mcp-apps/config-multiplex.yaml
```

This config serves two Apps-enabled servers behind one endpoint. Tool names are prefixed with their target (`map_`, `monitor_`) and `ui://` URIs are rewritten to carry the target (`ui://monitor+system-monitor/mcp-app.html`), so both servers' tools list, call, and render correctly.

The limitation to observe: a rendered app can itself call tools, using tool names hardcoded in the app's HTML. The system-monitor app calls `poll-system-stats` by its unprefixed name, which the gateway cannot route when multiplexing — the dashboard renders, but its live updates fail. The map app makes no app-initiated calls and works fully. If your host relies on app-initiated tool calls, use a single-target backend, or customize the app to call the prefixed name.
