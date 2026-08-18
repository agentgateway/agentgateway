#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLEANUP="${SCRIPT_DIR}/../../../cleanup-gke-campaign.sh"
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf -- "${TEMP_DIR}"' EXIT

mkdir -p "${TEMP_DIR}/bin"

cat >"${TEMP_DIR}/bin/kubectl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "kubectl $*" >>"${TEST_COMMAND_LOG}"
args=" $* "
if [[ "${args}" == *" cluster-info "* ]]; then
  exit 0
fi
if [[ "${args}" == *" get namespaces "* ]]; then
  [[ "${TEST_STORAGE_FAILURE:-false}" == "true" ]] && printf 'owned\ttest-campaign\n'
  exit 0
fi
if [[ "${args}" == *" get pvc "* ]]; then
  [[ "${TEST_STORAGE_FAILURE:-false}" == "true" ]] && printf 'model-pvc\tpv-bad\n'
  exit 0
fi
if [[ "${args}" == *" get pv "*"jsonpath={range .items"* ]]; then
  exit 0
fi
if [[ "${args}" == *" get pv pv-bad "*"jsonpath={.spec.claimRef.namespace}"* ]]; then
  printf 'owned'
  exit 0
fi
if [[ "${args}" == *" get pv pv-bad "*"jsonpath={.spec.csi.driver}"* ]]; then
  printf 'unexpected.csi.example'
  exit 0
fi
if [[ "${args}" == *" get pv pv-bad "* ]]; then
  exit 1
fi
if [[ "${args}" == *" get nodes "* ]]; then
  exit 0
fi
if [[ "${args}" == *" get configmap campaign-lock "* ]]; then
  printf 'test-campaign'
  exit 0
fi
exit 0
EOF

cat >"${TEMP_DIR}/bin/gcloud" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "gcloud $*" >>"${TEST_COMMAND_LOG}"
args=" $* "
if [[ "${args}" == *" container node-pools describe "*"format=value(name)"* ]]; then
  printf 'gpu-h100\n'
  exit 0
fi
if [[ "${args}" == *" container node-pools describe "*"format=value(instanceGroupUrls[])"* ]]; then
  printf 'https://www.googleapis.com/compute/v1/projects/test/zones/us-central1-a/instanceGroupManagers/test-gpu-group\n'
  exit 0
fi
if [[ "${args}" == *" compute instance-groups managed describe "* ]]; then
  printf '0\n'
  exit 0
fi
if [[ "${args}" == *" filestore instances list "* ]]; then
  printf '[]\n'
  exit 0
fi
exit 0
EOF

chmod +x "${TEMP_DIR}/bin/kubectl" "${TEMP_DIR}/bin/gcloud"

run_cleanup() {
  PATH="${TEMP_DIR}/bin:${PATH}" \
  TEST_COMMAND_LOG="${TEMP_DIR}/commands.log" \
  BENCHMARK_KUBE_CONTEXT=gke_test_us-central1-a_cluster \
  BENCHMARK_CAMPAIGN_ID=test-campaign \
  BENCHMARK_GKE_GPU_NODEPOOL=gpu-h100 \
  BENCHMARK_GKE_CLEANUP_TIMEOUT=1 \
  TEST_STORAGE_FAILURE="${1:-false}" \
    "${CLEANUP}"
}

: >"${TEMP_DIR}/commands.log"
run_cleanup false
grep -q 'container clusters resize cluster .*--num-nodes 0' "${TEMP_DIR}/commands.log"
grep -q 'delete configmap campaign-lock' "${TEMP_DIR}/commands.log"

: >"${TEMP_DIR}/commands.log"
if run_cleanup true; then
  echo "cleanup unexpectedly accepted an unowned CSI volume" >&2
  exit 1
fi
grep -q 'container clusters resize cluster .*--num-nodes 0' "${TEMP_DIR}/commands.log"
if grep -q 'delete configmap campaign-lock' "${TEMP_DIR}/commands.log"; then
  echo "failed cleanup unexpectedly released the campaign lock" >&2
  exit 1
fi

echo "GKE campaign cleanup tests passed"
