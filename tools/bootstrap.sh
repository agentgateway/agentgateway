#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
export PATH="${REPO_ROOT}/tools:${PATH}"

require() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing prerequisite: $1" >&2
    echo "Run 'make doctor' for the full prerequisite check." >&2
    exit 1
  }
}

require cargo
require go
require npm

(
  cd "${REPO_ROOT}"
  cargo fetch
  go mod download all
)

(
  cd "${REPO_ROOT}/tools"
  go mod download all
)

npm --prefix "${REPO_ROOT}/ui" ci

echo "Bootstrap complete."
echo "Run 'make doctor' to verify host prerequisites and 'make test' to validate the repo."
