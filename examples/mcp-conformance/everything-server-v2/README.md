# v2 everything-server (modern-draft suite conformance upstream)

`everything-server.ts` is vendored from the MCP TypeScript SDK
(`modelcontextprotocol/typescript-sdk`, MIT), `test/conformance/src/everythingServer.ts`.
It is a v2-SDK (`2.0.0-beta.2`) server implementing the 2026-07-28 draft.

## Why this is the modern-draft upstream

The `modern-draft` suite runs against this server, not the reference **v1** everything-server
bundled in the framework clone (`$MCP_CONFORMANCE_DIR/examples/servers/typescript`).
The v1 server does not enforce the prerequisites the current suite depends on: it
grades some scenarios "not testable" and passes SEP-2322 input-required flows
without checking the negotiated client capabilities. The v2 server enforces those
checks. It also bundles fixtures the v1 server lacks:

- **SEP-2243** `test_x_mcp_header`: enables `http-custom-header-server-validation`.
- **SEP-2575** diagnostic tools (`test_missing_capability`, …).

It must be the **sole** MCP target: the gateway prefixes every tool name when
multiplexing (`>1` target), which breaks conformance's `Mcp-Name` matching.
(The `active`/legacy suite still runs against the v1 reference.)

## Run standalone

```bash
npm install
PORT=3001 npm start   # serves http://localhost:3001/mcp
```

## In the harness

`mcp_conformance_modern_draft` boots this server automatically and grades against
`baseline-modern-draft.yml`:

```bash
cd examples/mcp-conformance/everything-server-v2 && npm install && cd -
export MCP_CONFORMANCE=1 MCP_CONFORMANCE_DIR=~/oss/mcp-conformance RUST_MIN_STACK=16777216
cargo test -p agentgateway --test mcp_conformance mcp_conformance_modern_draft -- --ignored --nocapture
```

## Re-vendoring

See the header of `everything-server.ts` for the exact `gh api` command and the
upstream commit it was taken from.
