# MCP conformance harness

This opt-in harness runs the official MCP Conformance Framework in server mode
against two topologies for each suite:

```
direct:   framework -> TypeScript SDK conformance server
gateway:  framework -> agentgateway -> TypeScript SDK conformance server
```

The direct control makes failures attributable per check. A direct pass plus a
gateway failure is gateway-attributed; a failure in both is control-blocked and
inconclusive; a direct failure plus a gateway pass means the gateway changed
behaviour. A missing, `SKIPPED`, or `INFO` direct result paired with a gateway
failure is marked `investigate`.

> !NOTE
> This harness covers only the framework's server command. The client command
expects a scenario-dispatching client process, which AgentGateway cannot expose
without a dedicated shim; its client scenarios, including OAuth and DPoP, are
therefore out of scope. The authorization command tests an OAuth authorization
server and does not apply to AgentGateway.

## Setup and runs

Prerequisites: `git`, `cargo`, `python3`, `node`, `npm` (installs the
framework), and `pnpm` (installs and builds the TypeScript SDK) on PATH. Every
make target checks all of these up front and fails listing what is missing.
Use node 24 LTS — an upstream constraint of the pinned conformance framework,
not of this harness: under node 26 the framework crashes mid-suite with an
unhandled promise rejection on direct SSE scenarios
(`server-sse-multiple-streams`), which surfaces as an incomplete result set.
Re-check when bumping `framework.sha`.

`framework.sha` pins the runner and `typescript-sdk.sha` pins the server. Start
in the agentgateway repo root; the first two lines anchor every later path, so
each step keeps working as the commands change directory:

```sh
AGENTGATEWAY="$PWD"
PINS="$AGENTGATEWAY/crates/agentgateway/tests/conformance"

git clone https://github.com/modelcontextprotocol/conformance.git /tmp/mcp-conformance
cd /tmp/mcp-conformance
git checkout "$(cat "$PINS/framework.sha")"
npm ci
export MCP_CONFORMANCE_DIR="$PWD"

git clone https://github.com/modelcontextprotocol/typescript-sdk.git /tmp/mcp-typescript-sdk
cd /tmp/mcp-typescript-sdk
git checkout "$(cat "$PINS/typescript-sdk.sha")"
pnpm install --frozen-lockfile
pnpm build:all
export MCP_TYPESCRIPT_SDK_DIR="$PWD"

cd "$AGENTGATEWAY"
```

The variables and exports are per-shell: in a new terminal, re-export
`MCP_CONFORMANCE_DIR` and `MCP_TYPESCRIPT_SDK_DIR` before running the make
targets (they fail with instructions when either is missing).

`make mcp-conformance` runs everything: all graded suites through both
topologies, including the JSON Schema scenario selected from upstream's pending
suite as additional coverage. Its output root is what the report consumes:

```sh
make mcp-conformance OUT=target/mcp-conformance/full
MCP_CONFORMANCE_OUT=target/mcp-conformance/full make mcp-conformance-report
```

Without `OUT=` it creates a unique root under `target/mcp-conformance/` and, on
success, prints the exact report command. Each run needs a root without prior
results for its suites. The target exits successfully only when results match
the expected-failure baselines: accepted gaps pass, while new failures, stale
expected failures, and incomplete output fail it. The target is CI-ready, but
no workflow invokes it yet. A capture does not grade against a baseline and
retains raw data for diagnosis:

```sh
make mcp-conformance-capture SUITE=2025-11-25
make mcp-conformance-capture SUITE=2026-07-28
make mcp-conformance-capture SUITE=pending
```

Each capture creates its own output root. Pass `OUT=<dir>` to collect several
suites in one root for side-by-side inspection; re-capturing a suite already
present there is refused, so remove its `direct-*`/`gateway-*` directories or
use a fresh root:

```sh
make mcp-conformance-capture SUITE=2026-07-28 OUT=target/mcp-conformance/triage
make mcp-conformance-capture SUITE=2025-11-25 OUT=target/mcp-conformance/triage
```

To check whether the pinned reference fixture can run one pending scenario,
use the direct availability probe. It exits nonzero when that scenario does not
pass directly, so a passing probe is evidence that its fixture prerequisite is
available rather than a gateway result:

```sh
make mcp-conformance-pending-availability SCENARIO=json-schema-2020-12
```

`mcp-conformance-capture` also accepts `SCENARIO=<name>` to capture direct and
gateway results for one scenario without grading it.

`MCP_CONFORMANCE_OUT=<run-root> make mcp-conformance-report` writes
`status.json`, renders `status.md`, and appends `status-history.json`. It
consumes the complete graded output of `make mcp-conformance` — capture roots
lack the graded pending coverage and may be partial. Reporting requires a clean
committed gateway worktree so the recorded SHA is meaningful. Each status entry
records both the framework and TypeScript SDK server pins.

## Suites

Public suite names use protocol revisions rather than the framework's selector
aliases:

- `2025-11-25` (31, upstream `active`): the broadly deployed stateful MCP
  baseline before the 2026 revision. The pinned framework calls this lane
  `active`; the harness names it by the protocol revision it exercises.
- `2026-07-28` (19): the scenarios introduced by the released 2026-07-28
  revision. The pinned framework selects them as `draft`, so the harness maps
  the public name to the upstream selector (`RELEASE_2026_07_28` in
  `mcp_conformance.rs`). The suite name is
  shared vocabulary across expected-failure files, output directories,
  `SUITE=`, and status reports; naming it by revision date keeps all of them
  stable when upstream renames its selector.
- `pending` (14): upstream's deferral bucket, not a protocol revision or a
  draft-spec lane. It includes scenarios whose framework or generic reference
  fixture is not ready to support a meaningful grade. The two HTTP validation
  scenarios therefore appear in both `2026-07-28` and pending. Most pending
  scenarios are capture-only; `gatedPendingScenarios` is the reviewed
  allowlist for additional coverage. The inventory generator rejects an entry
  absent from upstream's `pending` selector, so a pin bump cannot silently
  retarget it. At this pin it contains `json-schema-2020-12`, which
  runs as `pending-json-schema-2020-12` with the upstream `pending` selector
  narrowed to that scenario. Its direct control and gateway run are graded, so
  a gateway-only failure is reported as a 2025-11-25 gap and fails
  `mcp-conformance`.

The framework's remaining selectors add no coverage: `core` aliases upstream
`active` (`2025-11-25` here), and `all` combines the three lanes. A merged lane
would collapse per-suite expected-failure gating.

## Inventory and expected failures

`suite-inventory.json` records the framework pin, each upstream suite's exact
scenario set, and reviewed additional pending coverage. Regenerate it after a
framework pin bump:

```sh
MCP_CONFORMANCE_DIR=<clone> make mcp-conformance-inventory
```

Preflight re-runs the same generator against the clone and requires
byte-identical output, so a pin bump cannot silently change coverage.

The `expected-failures-*.yml` files are not test manifests: the framework
discovers and runs every scenario in `suite-inventory.json`. They only name
currently accepted failures. The direct files are empty controls; the gateway
files contain the known gaps. Expected failures use the framework's parser,
not a local YAML approximation. The two TypeScript helpers invoke the pinned
framework's suite selectors (`generate-inventory.ts`) and expected-failures
parser (`parse-expected-failures.ts`); the reporting and result handling remain
in Python. Entries are either a whole scenario
(`server-stateless`) or one check
(`server-stateless:sep-2575-server-implements-discover`); there must be **no
space after the colon**. A whole-scenario and a per-check entry cannot coexist.
Only a demonstrated non-INFO success makes a per-check entry stale; absent and
`SKIPPED` checks are deliberately no-signal. `wire-schema-valid` checks are
ordinary checks and may be the failure source.

`expected-failure-rationales.json` holds status-visible context for selected
baseline entries. Each key must name an entry from the corresponding official
expected-failures file; it records a short `kind` and `summary` without changing
the framework-owned baseline grammar. The report includes this rationale in
both `status.json` and `status.md`.

There are independent direct and gateway expected-failure files. An entry is present only
when that topology actually fails; comments classify it as control-blocked,
gateway gap, or intentional gateway behaviour. Missing `checks.json` is fatal:
the framework can synthesize a failure for a throwing scenario while writing no
result file, so a partial output must never be accepted as a graded run.

Most pending scenarios are intentionally ungated because the generic fixture
cannot exercise them meaningfully. Use capture to inspect those scenarios. A
pending scenario can move into `gatedPendingScenarios` only after direct and
gateway captures pass against the pinned fixture. Start with
`mcp-conformance-pending-availability SCENARIO=<name>` to establish the direct
fixture prerequisite; it then receives its own expected-failure files and
strict additional-coverage run rather than a silent expected failure.

## Updating the pin

Change either pin, install the matching clone, capture direct and gateway
results, then re-triage all four expected-failure files. When updating
`framework.sha`, also regenerate `suite-inventory.json`
(`make mcp-conformance-inventory`) and review the diff. The inventory/preflight
checks intentionally fail until this is done.
Raw results are under
`results/server-<scenario>-<timestamp>/checks.json` within each topology/suite
output directory.
