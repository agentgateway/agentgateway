#!/usr/bin/env bash

set -o errexit
set -o pipefail
set -o nounset

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
cd "${REPO_ROOT}"

TIMINGS_FILE="${REPO_ROOT}/controller/_test/ci-step-timings.log"
PROFILE="${DEV_ENV_PROFILE:-ci}"
TEST_MODE="${TEST_MODE:-unknown}"

mkdir -p "$(dirname "${TIMINGS_FILE}")"
: >"${TIMINGS_FILE}"

maybe_prefix() {
  if command -v ts >/dev/null 2>&1; then
    ts "$1:"
  else
    cat
  fi
}

run_step() {
  local step_name="$1"
  shift

  local start_seconds
  local end_seconds
  local elapsed_seconds
  local rc

  start_seconds="$(date +%s)"
  echo "==> Step started: ${step_name}" >&2

  if "$@" |& maybe_prefix "${step_name}"; then
    rc=0
  else
    rc=$?
  fi

  end_seconds="$(date +%s)"
  elapsed_seconds=$((end_seconds - start_seconds))
  printf '%s: %ss\n' "${step_name}" "${elapsed_seconds}" >>"${TIMINGS_FILE}"

  if [[ "${rc}" -ne 0 ]]; then
    echo "Step failed: ${step_name} (exit ${rc})" >&2
  else
    echo "==> Step completed: ${step_name} (${elapsed_seconds}s)" >&2
  fi

  return "${rc}"
}

step_env_up() {
  TEST_MODE="${TEST_MODE}" ./tools/dev-env.sh --profile "${PROFILE}" up -- "$@"
}

step_warm_test() {
  case "${TEST_MODE}" in
    e2e)
      CGO_ENABLED=0 go test -tags=e2e -exec=true -toolexec=./tools/go-compile-without-link -vet=off ./controller/test/e2e/tests
      ;;
    conformance)
      :
      ;;
    *)
      :
      ;;
  esac
}

main() {
  echo "Timings will be written to: ${TIMINGS_FILE}"
  run_step "env-up" step_env_up "$@"
  run_step "warm-test" step_warm_test
}

main "$@"
