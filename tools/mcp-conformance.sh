#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
CONFORMANCE_DIR="$ROOT_DIR/crates/agentgateway/tests/conformance"
FRAMEWORK_PIN="$CONFORMANCE_DIR/framework.sha"
TYPESCRIPT_SDK_PIN="$CONFORMANCE_DIR/typescript-sdk.sha"

cd "$ROOT_DIR"

usage() {
  cat <<'EOF'
Usage: tools/mcp-conformance.sh <run|capture|availability|inventory|report> [arguments]

  run [out]                         Grade all suites and reviewed additional coverage; the
                                    result is the input for report.
  capture <suite> [scenario] [out]  Diagnose: keep one suite's raw direct and gateway results,
                                    ungraded, tolerating crashed or partial scenarios. A shared
                                    [out] directory collects several suites.
  availability <scenario>           Check whether the pinned fixture passes a pending scenario directly.
  inventory                         Regenerate suite-inventory.json from MCP_CONFORMANCE_DIR.
  report                            Generate status files from MCP_CONFORMANCE_OUT.

Run these through make to use the documented public interface.
EOF
}

require_commands() {
  local missing=()
  # npm and pnpm are setup-time tools (framework install; TypeScript SDK
  # install/build), required again at every pin bump — check them here so the
  # first make target reports everything at once.
  for command in git cargo python3 node npm pnpm; do
    command -v "$command" >/dev/null || missing+=("$command")
  done
  if ((${#missing[@]})); then
    cat >&2 <<EOF
Missing required command(s): ${missing[*]}
See crates/agentgateway/tests/conformance/README.md#setup-and-runs.
EOF
    exit 1
  fi
  local node_version
  node_version="$(node --version)"
  if [[ ! "$node_version" =~ ^v24\. ]]; then
    cat >&2 <<EOF
Node.js 24 LTS is required by the pinned MCP conformance framework; found $node_version.
See crates/agentgateway/tests/conformance/README.md#setup-and-runs.
EOF
    exit 1
  fi
}

require_var() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    case "$name" in
      MCP_CONFORMANCE_DIR)
        cat >&2 <<EOF
MCP_CONFORMANCE_DIR must name a clean MCP Conformance Framework clone at $(cat "$FRAMEWORK_PIN").
See crates/agentgateway/tests/conformance/README.md#setup-and-runs.

Example:
  export MCP_CONFORMANCE_DIR=/path/to/mcp-conformance
  export MCP_TYPESCRIPT_SDK_DIR=/path/to/mcp-typescript-sdk
  make mcp-conformance
EOF
        ;;
      MCP_TYPESCRIPT_SDK_DIR)
        cat >&2 <<EOF
MCP_TYPESCRIPT_SDK_DIR must name a clean TypeScript SDK clone at $(cat "$TYPESCRIPT_SDK_PIN").
See crates/agentgateway/tests/conformance/README.md#setup-and-runs.

Example:
  export MCP_TYPESCRIPT_SDK_DIR=/path/to/mcp-typescript-sdk
  make mcp-conformance
EOF
        ;;
      MCP_CONFORMANCE_OUT)
        cat >&2 <<'EOF'
MCP_CONFORMANCE_OUT must name the output directory printed by make mcp-conformance.
See crates/agentgateway/tests/conformance/README.md#setup-and-runs.
EOF
        ;;
      *)
        echo "set $name" >&2
        ;;
    esac
    exit 1
  fi
}

require_clean_checkout() {
  local variable="$1"
  local directory="${!variable}"
  if [[ -n "$(git -C "$directory" status --porcelain --untracked-files=no)" ]]; then
    cat >&2 <<EOF
$variable has tracked changes: $directory
Inspect them with: git -C "$directory" status --short
EOF
    exit 1
  fi
}

require_pinned_checkout() {
  local variable="$1"
  local pin="$2"
  local directory="${!variable}"
  local actual revision
  revision="$(cat "$pin")"
  actual="$(git -C "$directory" rev-parse HEAD)"
  if [[ "$actual" != "$revision" ]]; then
    cat >&2 <<EOF
$variable is at $actual; expected $revision.
Use a clean clone at the pinned revision. See crates/agentgateway/tests/conformance/README.md#setup-and-runs.
EOF
    exit 1
  fi
  require_clean_checkout "$variable"
}

require_framework() {
  require_var MCP_CONFORMANCE_DIR
  require_pinned_checkout MCP_CONFORMANCE_DIR "$FRAMEWORK_PIN"
}

require_typescript_sdk() {
  require_var MCP_TYPESCRIPT_SDK_DIR
  require_pinned_checkout MCP_TYPESCRIPT_SDK_DIR "$TYPESCRIPT_SDK_PIN"
}

suite_scenarios() {
  python3 -c '
import json
import sys
for scenario in json.load(open(sys.argv[1]))["suites"][sys.argv[2]]:
    print(scenario)
' "$CONFORMANCE_DIR/suite-inventory.json" "$1"
}

require_scenario_in_suite() {
  local suite="$1"
  local scenario="$2"
  if [[ -z "$scenario" ]]; then
    return
  fi
  # grep without -q reads all input, so the writer never dies of SIGPIPE under pipefail.
  if ! suite_scenarios "$suite" | grep -Fx -- "$scenario" >/dev/null; then
    cat >&2 <<EOF
$scenario is not in the $suite suite recorded by suite-inventory.json.
Use make mcp-conformance-capture SUITE=$suite without SCENARIO to capture the full suite.
EOF
    exit 1
  fi
}

require_pending_scenario() {
  local scenario="$1"
  if [[ -n "$scenario" ]]; then
    require_scenario_in_suite pending "$scenario"
    return
  fi
  cat >&2 <<EOF
Set SCENARIO to a pending scenario.

Example:
  make mcp-conformance-pending-availability SCENARIO=json-schema-2020-12

Pending scenarios recorded at this pin:
EOF
  suite_scenarios pending | sed 's/^/  /' >&2
  exit 1
}

new_output_dir() {
  local kind="$1"
  mkdir -p "$ROOT_DIR/target/mcp-conformance"
  mktemp -d "$ROOT_DIR/target/mcp-conformance/$kind.XXXXXX"
}

# Use the caller-named directory when given, a fresh temp root otherwise.
# Absolutized because the test binary runs from the package directory, not the
# repo root. Per-(topology, suite) freshness is enforced by the Rust harness.
resolve_output_dir() {
  local out="$1"
  local kind="$2"
  if [[ -n "$out" ]]; then
    mkdir -p "$out"
    (cd "$out" && pwd)
  else
    new_output_dir "$kind"
  fi
}

run_test() {
  local output="$1"
  shift
  MCP_CONFORMANCE=1 MCP_CONFORMANCE_OUT="$output" \
    cargo test -p agentgateway --test mcp_conformance -- --ignored --exact --test-threads=1 "$@"
}

run() {
  require_commands
  local out="${1:-}"
  require_framework
  require_typescript_sdk
  python3 -m unittest discover -s "$CONFORMANCE_DIR" -p '*_tests.py'

  local output
  output="$(resolve_output_dir "$out" run)"
  echo "MCP conformance output: $output"
  # One cargo invocation: the tests share a process, so the expensive preflight
  # (git + tsx) runs once; --test-threads=1 keeps the suites serialized.
  run_test "$output" \
    direct_2025_11_25 \
    gateway_2025_11_25 \
    direct_2026_07_28 \
    gateway_2026_07_28 \
    direct_pending_json_schema_2020_12 \
    gateway_pending_json_schema_2020_12
  echo "Report: MCP_CONFORMANCE_OUT=$output make mcp-conformance-report"
}

capture() {
  require_commands
  local suite="${1:-}"
  local scenario="${2:-}"
  local out="${3:-}"
  case "$suite" in
    2025-11-25|2026-07-28|pending) ;;
    *)
      cat >&2 <<'EOF'
Set SUITE to 2025-11-25, 2026-07-28, or pending.

Example:
  make mcp-conformance-capture SUITE=pending SCENARIO=json-schema-2020-12
EOF
      exit 1
      ;;
  esac
  require_scenario_in_suite "$suite" "$scenario"

  require_framework
  require_typescript_sdk

  local output
  output="$(resolve_output_dir "$out" capture)"
  echo "MCP conformance capture output: $output"
  for topology in direct gateway; do
    MCP_CONFORMANCE_SUITE="$suite" MCP_CONFORMANCE_SCENARIO="$scenario" \
      MCP_CONFORMANCE_TOPOLOGY="$topology" \
      run_test "$output" capture
  done
}

availability() {
  require_commands
  local scenario="${1:-}"
  require_pending_scenario "$scenario"

  require_framework
  require_typescript_sdk

  local output
  output="$(new_output_dir pending-availability)"
  MCP_CONFORMANCE_SCENARIO="$scenario" run_test "$output" pending_fixture_available
  echo "Pending fixture availability output: $output"
}

inventory() {
  require_commands
  if [[ -z "${MCP_CONFORMANCE_DIR:-}" ]]; then
    cat >&2 <<'EOF'
MCP_CONFORMANCE_DIR must name a clean MCP Conformance Framework clone.
Use it to regenerate the inventory after updating framework.sha. See crates/agentgateway/tests/conformance/README.md#updating-the-pin.

Example:
  MCP_CONFORMANCE_DIR=/path/to/mcp-conformance make mcp-conformance-inventory
EOF
    exit 1
  fi
  require_clean_checkout MCP_CONFORMANCE_DIR

  local inventory="$CONFORMANCE_DIR/suite-inventory.json"
  local temporary="$inventory.tmp"
  "$MCP_CONFORMANCE_DIR/node_modules/.bin/tsx" \
    "$CONFORMANCE_DIR/generate-inventory.ts" "$MCP_CONFORMANCE_DIR" > "$temporary"
  mv "$temporary" "$inventory"
  echo "wrote crates/agentgateway/tests/conformance/suite-inventory.json at $(git -C "$MCP_CONFORMANCE_DIR" rev-parse HEAD)"
}

report() {
  require_commands
  # The report runs the framework clone's parser and records both clone SHAs as
  # provenance, so the pins must still hold at report time, not just at run time.
  require_framework
  require_typescript_sdk
  require_var MCP_CONFORMANCE_OUT
  if [[ -n "$(git -C "$ROOT_DIR" status --porcelain --untracked-files=no)" ]]; then
    cat >&2 <<'EOF'
Reporting requires a clean committed gateway worktree so status history records a meaningful gateway SHA.
Inspect the worktree with: git status --short
EOF
    exit 1
  fi

  python3 "$CONFORMANCE_DIR/report.py" \
    --inventory "$CONFORMANCE_DIR/suite-inventory.json" \
    --framework-dir "$MCP_CONFORMANCE_DIR" \
    --out "$MCP_CONFORMANCE_OUT" \
    --framework "$(git -C "$MCP_CONFORMANCE_DIR" rev-parse HEAD)" \
    --typescript-sdk "$(git -C "$MCP_TYPESCRIPT_SDK_DIR" rev-parse HEAD)" \
    --gateway-sha "$(git -C "$ROOT_DIR" rev-parse HEAD)" \
    --gateway-ref "$(git -C "$ROOT_DIR" branch --show-current)"
}

case "${1:-}" in
  run)
    shift
    run "$@"
    ;;
  capture)
    shift
    capture "$@"
    ;;
  availability)
    shift
    availability "$@"
    ;;
  inventory)
    inventory
    ;;
  report)
    report
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
