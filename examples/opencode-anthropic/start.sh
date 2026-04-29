#!/usr/bin/env bash
# Start agentgateway for the opencode-anthropic demo.
#
# Usage:
#   export ANTHROPIC_API_KEY="sk-ant-..."
#   ./examples/opencode-anthropic/start.sh

set -euo pipefail

if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
  echo "Error: ANTHROPIC_API_KEY is not set." >&2
  echo "  export ANTHROPIC_API_KEY=\"sk-ant-...\"" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${REPO_ROOT}/target/release/agentgateway"

if [[ ! -x "$BINARY" ]]; then
  echo "Release binary not found. Building..." >&2
  cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml"
fi

echo "Starting agentgateway on http://localhost:3000 ..."
exec "$BINARY" --file "${SCRIPT_DIR}/gateway.yaml"
