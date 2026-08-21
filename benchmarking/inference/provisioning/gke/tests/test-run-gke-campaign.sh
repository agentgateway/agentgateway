#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CAMPAIGN_SCRIPT="$(cd "${SCRIPT_DIR}/../../.." && pwd)/run-gke-campaign.sh"
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf -- "${TEMP_DIR}"' EXIT
mkdir -p "${TEMP_DIR}/bin"

cat >"${TEMP_DIR}/bin/make" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
target="${!#}"
printf 'target=%s treatment=%s campaign=%s comparisons=%s\n' \
  "${target}" "${BENCHMARK_TREATMENT:-}" "${BENCHMARK_CAMPAIGN_ID:-}" \
  "${BENCHMARK_COMPARISONS:-}" >>"${TEST_COMMAND_LOG}"
if [[ "${target}" == "${TEST_FAIL_TARGET:-none}" ]]; then
  exit "${TEST_FAIL_STATUS:-43}"
fi
if [[ "${target}" == benchmark && \
      "${BENCHMARK_TREATMENT:-}" == "${TEST_FAIL_TREATMENT:-none}" ]]; then
  exit 42
fi
exit 0
EOF

cat >"${TEMP_DIR}/bin/kubectl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'kubectl %s\n' "$*" >>"${TEST_COMMAND_LOG}"
args=" $* "
if [[ "${args}" == *" create namespace "* ]]; then
  printf 'apiVersion: v1\nkind: Namespace\nmetadata:\n  name: benchmark-secrets\n'
elif [[ "${args}" == *" apply -f - "* ]]; then
  while IFS= read -r _; do :; done
elif [[ "${args}" == *" get secret "* ]]; then
  [[ "${TEST_SECRET_EXISTS:-true}" == true ]] || exit 1
  [[ "${args}" != *" --template="* ]] || printf 'hf_test'
elif [[ "${args}" == *" create secret generic "* ]]; then
  while IFS= read -r _; do :; done
  printf 'apiVersion: v1\nkind: Secret\nmetadata:\n  name: llm-d-hf-token\n'
fi
exit 0
EOF

chmod +x "${TEMP_DIR}/bin/make" "${TEMP_DIR}/bin/kubectl"
export PATH="${TEMP_DIR}/bin:/usr/bin:/bin"
export BENCHMARK_MAKE_BIN="${TEMP_DIR}/bin/make"
export BENCHMARK_GKE_PROJECT=test-project
export BENCHMARK_GKE_NODE_SERVICE_ACCOUNT=default
export TEST_COMMAND_LOG="${TEMP_DIR}/commands.log"

: >"${TEST_COMMAND_LOG}"
TEST_SECRET_EXISTS=false HF_TOKEN=hf_not_logged \
BENCHMARK_CAMPAIGN_ID=test-success-campaign \
  "${CAMPAIGN_SCRIPT}" >"${TEMP_DIR}/success.log"
[[ "$(grep -c '^target=benchmark-gke-gpu-up ' "${TEST_COMMAND_LOG}")" == 3 ]]
[[ "$(grep -c '^target=benchmark ' "${TEST_COMMAND_LOG}")" == 3 ]]
grep -q '^target=benchmark treatment=service ' "${TEST_COMMAND_LOG}"
grep -q '^target=benchmark treatment=agentgateway-standalone ' "${TEST_COMMAND_LOG}"
grep -q '^target=benchmark treatment=agentgateway-gateway ' "${TEST_COMMAND_LOG}"
grep -q '^target=benchmark-report .*comparisons=service:agentgateway-standalone service:agentgateway-gateway$' \
  "${TEST_COMMAND_LOG}"
[[ "$(tail -n 1 "${TEST_COMMAND_LOG}")" == target=benchmark-gke-cleanup* ]]
grep -q 'create secret generic llm-d-hf-token' "${TEST_COMMAND_LOG}"
if grep -q 'hf_not_logged' "${TEST_COMMAND_LOG}"; then
  echo "HF token leaked into the campaign command log" >&2
  exit 1
fi

: >"${TEST_COMMAND_LOG}"
TEST_SECRET_EXISTS=true BENCHMARK_GKE_CLUSTER_LIFECYCLE=destroy \
BENCHMARK_CAMPAIGN_ID=test-ephemeral-success \
  "${CAMPAIGN_SCRIPT}" >"${TEMP_DIR}/ephemeral-success.log"
grep -q '^target=benchmark-report ' "${TEST_COMMAND_LOG}"
[[ "$(tail -n 2 "${TEST_COMMAND_LOG}" | head -n 1)" == target=benchmark-gke-cleanup* ]]
[[ "$(tail -n 1 "${TEST_COMMAND_LOG}")" == target=benchmark-gke-destroy* ]]

: >"${TEST_COMMAND_LOG}"
set +e
TEST_SECRET_EXISTS=true TEST_FAIL_TREATMENT=agentgateway-standalone \
BENCHMARK_GKE_CLUSTER_LIFECYCLE=destroy \
BENCHMARK_CAMPAIGN_ID=test-failed-campaign \
  "${CAMPAIGN_SCRIPT}" >"${TEMP_DIR}/failure.log" 2>&1
status=$?
set -e
[[ "${status}" == 42 ]]
[[ "$(grep -c '^target=benchmark-gke-gpu-up ' "${TEST_COMMAND_LOG}")" == 2 ]]
grep -q '^target=benchmark-gke-cleanup ' "${TEST_COMMAND_LOG}"
[[ "$(tail -n 1 "${TEST_COMMAND_LOG}")" == target=benchmark-gke-destroy* ]]
if grep -q '^target=benchmark-report ' "${TEST_COMMAND_LOG}"; then
  echo "failed campaign unexpectedly generated reports" >&2
  exit 1
fi

: >"${TEST_COMMAND_LOG}"
set +e
TEST_SECRET_EXISTS=true TEST_FAIL_TARGET=benchmark-gke-provision \
BENCHMARK_GKE_CLUSTER_LIFECYCLE=destroy TEST_FAIL_STATUS=43 \
BENCHMARK_CAMPAIGN_ID=test-provision-failure \
  "${CAMPAIGN_SCRIPT}" >"${TEMP_DIR}/provision-failure.log" 2>&1
status=$?
set -e
[[ "${status}" == 43 ]]
if grep -q '^target=benchmark-gke-cleanup ' "${TEST_COMMAND_LOG}"; then
  echo "provisioning failure unexpectedly ran campaign cleanup" >&2
  exit 1
fi
[[ "$(tail -n 1 "${TEST_COMMAND_LOG}")" == target=benchmark-gke-destroy* ]]
grep -q 'cluster lifecycle: destroy' "${TEMP_DIR}/provision-failure.log"

: >"${TEST_COMMAND_LOG}"
set +e
TEST_SECRET_EXISTS=true TEST_FAIL_TARGET=benchmark-gke-cleanup \
BENCHMARK_GKE_CLUSTER_LIFECYCLE=destroy TEST_FAIL_STATUS=44 \
BENCHMARK_CAMPAIGN_ID=test-cleanup-failure \
  "${CAMPAIGN_SCRIPT}" >"${TEMP_DIR}/cleanup-failure.log" 2>&1
status=$?
set -e
[[ "${status}" == 44 ]]
grep -q '^target=benchmark-gke-cleanup ' "${TEST_COMMAND_LOG}"
if grep -q '^target=benchmark-gke-destroy ' "${TEST_COMMAND_LOG}"; then
  echo "cleanup failure unexpectedly allowed cluster destruction" >&2
  exit 1
fi
grep -q 'refusing cluster deletion because campaign cleanup failed' \
  "${TEMP_DIR}/cleanup-failure.log"

if BENCHMARK_GKE_CLUSTER_LIFECYCLE=invalid \
  BENCHMARK_CAMPAIGN_ID=test-invalid-lifecycle \
  "${CAMPAIGN_SCRIPT}" >"${TEMP_DIR}/invalid-lifecycle.log" 2>&1; then
  echo "invalid cluster lifecycle was unexpectedly accepted" >&2
  exit 1
fi
grep -q 'BENCHMARK_GKE_CLUSTER_LIFECYCLE must be retain or destroy' \
  "${TEMP_DIR}/invalid-lifecycle.log"

echo "GKE campaign orchestration tests passed"
