#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

deps_started=0

cleanup() {
  local status=$?
  trap - EXIT INT TERM

  if (( deps_started )); then
    echo "Stopping validation dependencies..."
    "$SCRIPT_DIR/manage-validation-deps.sh" stop
  fi

  exit "$status"
}

trap cleanup EXIT INT TERM

cd "$REPO_ROOT"

if (( $# > 0 )); then
  config_files=("$@")
else
  config_files=()
  while IFS= read -r config_file; do
    config_files+=("$config_file")
  done < <(find examples -mindepth 2 -maxdepth 2 -name config.yaml -print | sort)
fi

if (( ${#config_files[@]} == 0 )); then
  echo "No config files found to validate."
  exit 0
fi

echo "Starting validation dependencies..."
"$SCRIPT_DIR/manage-validation-deps.sh" start
deps_started=1

# Example validation runs without shell setup, so provide a deterministic cookie
# secret for configs that enable browser auth.
export OIDC_COOKIE_SECRET="${OIDC_COOKIE_SECRET:-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}"

for config_file in "${config_files[@]}"; do
  echo "Validating $config_file"
  cargo run -- -f "$config_file" --validate-only
done
