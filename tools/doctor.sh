#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
export PATH="${REPO_ROOT}/tools:${PATH}"

FAILED=0

check_command() {
  local label="$1"
  local command="$2"
  local optional="${3:-false}"
  local output

  if output="$(bash -lc "${command}" 2>&1)"; then
    printf '[ok] %s: %s\n' "${label}" "$(printf '%s' "${output}" | head -n 1)"
    return
  fi

  if [[ "${optional}" == "true" ]]; then
    printf '[warn] %s: not available\n' "${label}"
    return
  fi

  printf '[missing] %s\n' "${label}"
  FAILED=1
}

check_command "Rust" "cargo --version"
check_command "Go" "go version"
check_command "Node" "node --version"
check_command "npm" "npm --version"
check_command "Docker" "docker version --format '{{.Server.Version}}'"
check_command "kubectl" "kubectl version --client=true"
check_command "Helm" "helm version --short"
check_command "kind" "kind version"
check_command "ctlptl" "ctlptl version"
check_command "jq" "jq --version"
check_command "Python" "python3 --version"
check_command "Tilt" "tilt version" "true"

if [[ "${FAILED}" -ne 0 ]]; then
  echo
  echo "Install the missing prerequisites, then re-run 'make doctor'." >&2
  exit 1
fi
