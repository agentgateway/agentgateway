# Example smoke tests

Functional smoke tests for the examples under [`examples/`](../../examples). One
generic runner starts the gateway against an example's shipped `config.yaml`,
sends real traffic through it, and asserts on the response.

This is distinct from config validation. Every example's config is already
validated on each PR/push (via `make test`, see
`crates/agentgateway/tests/validate_examples.rs`), which proves the config
parses and its references resolve. These smoke tests prove the example actually
*works*, and run on a schedule (`.github/workflows/examples.yml`) so they also
catch drift in external dependencies — npx MCP servers, provider API shapes —
not just changes in our own code.

## Adding coverage for an example

Drop a `smoke.yaml` next to the example's `config.yaml`. No Go required — the
runner discovers every `examples/*/smoke.yaml` automatically.

```yaml
# examples/<name>/smoke.yaml
readyAddr: 127.0.0.1:15021   # gateway readiness host:port (default shown)
env:                          # extra env for the gateway process (optional)
  SOME_KEY: value
mocks:                        # plain HTTP upstreams to start (reply 200 "ok")
  - listen: 127.0.0.1:8080
mockLLM: false                # start a mock LLM provider and point every model at it
probes:                       # requests to send once the gateway is ready
  - http:
      method: GET             # default GET
      url: http://127.0.0.1:3000/foo
      headers: {x-header: bar}
      body: ""
      expectStatus: 200       # default 200
      expectBody: "exact"     # optional exact match
      expectBodyContains: ["substr"]
  - mcp:
      endpoint: http://127.0.0.1:3000/mcp
      toolsContain: [echo]
      call:
        name: echo
        arguments: {message: hello}
        expectResultContains: [hello]
```

Each probe is exactly one of `http` or `mcp`. See the specs in
`examples/traffic-http`, `examples/mcp-basic`, and `examples/llm-basic` for
worked examples.

### mockLLM

`mockLLM: true` starts a local mock provider and rewrites the config so every
model's `baseUrl` resolves to it, so no live provider account is needed. The
mock reflects the request's prompt back as the completion, so a probe can send a
prompt and assert the same string comes back through the gateway — exercising
the example's model mapping and transformations without coupling to the mock.

Keep examples that need paid APIs or real cloud credentials out of these specs;
they belong in a separate, secret-gated workflow.

## Running locally

```bash
make build UI=0
make test-examples
# or target one example:
AGENTGATEWAY_BIN=$PWD/target/release/agentgateway \
  go test -tags examples -run TestExamples/mcp-basic -v ./test/examples/...
```

The runner resolves the gateway from `AGENTGATEWAY_BIN` (default
`target/release/agentgateway`), and skips if no binary is found. Requirements:
Go, plus `npx` (Node) for the MCP example. Tests are gated behind the `examples`
build tag so they stay out of the normal `go test ./...` run.
