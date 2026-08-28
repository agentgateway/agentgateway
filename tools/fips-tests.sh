#!/usr/bin/env bash
# Runs the focused FIPS test groups used by CI. Each filter is checked first so
# a renamed or removed test cannot make the corresponding test command pass
# without running anything.
set -euo pipefail

cd "$(dirname "$0")/.."

features="crypto-aws-lc-fips"

run_test_filter() {
  local filter="$1"
  local purpose="$2"
  local listed
  local count

  listed="$(cargo test -p agentgateway --no-default-features --features "$features" --lib "$filter" -- --list)"
  count="$(printf '%s\n' "$listed" | grep -cE ': test$' || true)"
  if [[ "$count" -eq 0 ]]; then
    echo "ERROR: no tests matched FIPS filter '$filter' ($purpose)." >&2
    return 1
  fi

  echo "Running $count test(s) matching '$filter' ($purpose)."
  cargo test -p agentgateway --no-default-features --features "$features" --lib "$filter"
}

run_test_filter 'crypto::' 'crypto provider behavior'
run_test_filter 'control::spiffe::' 'SPIFFE TLS behavior'

# Configuration-propagation tests must retain this prefix. These ensure user
# TLS selections reach the validated provider instead of silently bypassing it.
run_test_filter 'fips_config_' 'TLS configuration propagation'
