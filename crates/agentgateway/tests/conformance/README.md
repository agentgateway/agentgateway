# MCP conformance harness (opt-in)

Runs the upstream [MCP Conformance Test Framework][cf] in server mode against
agentgateway-as-proxy and grades the run against a committed expected-failures
baseline. The framework acts as an MCP *client*, fires spec scenarios at the
gateway, and the gateway forwards them to an everything-server upstream:

```
conformance (server mode) ──▶ agentgateway /mcp ──▶ everything-server /mcp
```

The upstream differs by suite. `active` runs against the framework clone's **v1**
reference server (`$MCP_CONFORMANCE_DIR/examples/servers/typescript`). `modern-draft`
runs against the vendored strict **v2** server
(`examples/mcp-conformance/everything-server-v2`): the v1 reference does not enforce
the SEP-2243/SEP-2322 prerequisites, so it passes input-required flows that still fail
against the stricter v2 server. See that dir's README for why.

`modern-draft` is our label for the modern (`2026-07-28` RC) scenario set; the upstream
framework knows it only as `draft`, so the test maps it to `--suite draft` at the CLI.

The test is `crates/agentgateway/tests/mcp_conformance.rs`. It is `#[ignore]`d and
gated on env vars, so it never runs in CI or a normal `cargo test`. Run it by hand
when changing MCP proxy behavior or tracking July-release (`2026-07-28`) progress.

[cf]: https://github.com/modelcontextprotocol/conformance

## Why a baseline ratchet

`--expected-failures <baseline>.yml` makes the run a two-way ratchet: a newly
failing scenario fails the run (a regression), and a newly passing one also
fails it (the baseline entry is now stale and must be removed). A scenario counts
as failing if any of its checks is `FAILURE` or `WARNING`. So each committed
`baseline-<suite>.yml` lists the scenarios that fail today. It is the
progress meter, not a mute list.

Two suites are tracked:

- `baseline-active.yml`: the `active` suite (dated releases up to `2025-11-25`):
  the downstream protocol the gateway supports today. This is the regression guard.
- `baseline-modern-draft.yml`: the `modern-draft` suite (the `2026-07-28` RC) against
  the strict v2 server: the July-release target.

Every entry in both baselines is a gateway-side gap; the reference upstream
behaves correctly. See the annotations in each file for the per-scenario cause.

## Prerequisites

1. A clone of the conformance framework, `npm install`ed in both the repo root
   and the everything-server subdir:

   ```bash
   git clone https://github.com/modelcontextprotocol/conformance ~/oss/mcp-conformance
   cd ~/oss/mcp-conformance && npm install
   (cd examples/servers/typescript && npm install)
   export MCP_CONFORMANCE_DIR=~/oss/mcp-conformance
   ```

   Baselines here were validated against framework `v0.2.0-alpha.9` plus
   conformance PR #392 at `f2fa81f`.

   The `modern-draft` suite additionally needs the vendored v2 server built once:

   ```bash
   (cd examples/mcp-conformance/everything-server-v2 && npm install)
   ```

2. `RUST_MIN_STACK=16777216` (16 MiB) is required. The MCP-proxy path enters
   the large `make_backend_call` async fn twice on one tokio data-plane worker
   (once for the inbound `Backend::MCP` arm, then again when the MCP upstream
   forwards via `PolicyClient`). In a debug build the combined frame size
   overflows the data-plane worker's default 2 MiB stack and the process aborts
   mid-scenario. Bumping the stack to 16 MiB avoids the overflow. This is a
   debug-build worker-stack issue only. Release builds use optimized
   (smaller) frames and are unaffected; no production code change is needed for the
   harness. Shrinking the `make_backend_call` frame on the MCP path is tracked as a
   separate follow-up.

## Run it

```bash
export MCP_CONFORMANCE=1
export MCP_CONFORMANCE_DIR=~/oss/mcp-conformance
export RUST_MIN_STACK=16777216

# both suites
cargo test --test mcp_conformance -- --ignored --nocapture

# one suite
cargo test --test mcp_conformance mcp_conformance_active -- --ignored --nocapture
cargo test --test mcp_conformance mcp_conformance_modern_draft -- --ignored --nocapture
```

The test boots the everything-server and the gateway on ephemeral ports itself; you
do not start them manually. A green run means the suite matched its baseline exactly.

## Modern (`2026-07-28`) downstream

The gateway accepts `2026-07-28` downstream natively (rmcp 2.0 / #2365), so the
`modern-draft` suite exercises the real modern path (stateless lifecycle, SEP-2243
headers, caching) with no flag, against the strict v2 everything-server.
`baseline-modern-draft.yml` records the remaining gateway gaps; the `active`/legacy
suite is the regression guard for dated releases.

## Regenerate a baseline

After an intended behavior change (a gap closed, or a new scenario added upstream),
regenerate rather than hand-editing:

1. Start the everything-server and a gateway in front of it (mirror the topology in
   `mcp_conformance.rs`, or use `examples/mcp-conformance/config.yaml`). Remember
   `RUST_MIN_STACK=16777216` on the gateway.
2. Run the suite against an empty baseline (`server: []`) with structured output:

   ```bash
   cd "$MCP_CONFORMANCE_DIR"
   printf 'server: []\n' > /tmp/empty.yml
   node_modules/.bin/tsx src/index.ts server \
     --url http://127.0.0.1:<gw-port>/mcp \
     --suite draft \
     --expected-failures /tmp/empty.yml \
     -o /tmp/conf-results
   ```

3. The run prints an "Unexpected failures" list. Those scenario names are the
   new baseline. Inspect each failure's cause in
   `/tmp/conf-results/<scenario>/checks.json` (the `errorMessage` per `FAILURE`/
   `WARNING` check) and copy the names into the `server:` key of the baseline,
	   annotated with the gateway-side cause (keep the existing grouping style).
4. Re-run with the updated baseline and confirm exit 0.
