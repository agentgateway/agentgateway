#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROVISION_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf -- "${TEMP_DIR}"' EXIT
mkdir -p "${TEMP_DIR}/bin" "${TEMP_DIR}/state"

if PATH=/usr/bin:/bin bash "${PROVISION_DIR}/provision.sh" plan \
  >"${TEMP_DIR}/missing-tool.log" 2>&1; then
  echo "provisioning unexpectedly accepted a missing gcloud CLI" >&2
  exit 1
fi
grep -q 'gcloud is required but was not found in PATH' "${TEMP_DIR}/missing-tool.log"

cat >"${TEMP_DIR}/bin/gke-gcloud-auth-plugin" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF

cat >"${TEMP_DIR}/bin/sleep" <<'EOF'
#!/usr/bin/env bash
/bin/sleep 1
EOF

cat >"${TEMP_DIR}/bin/kubectl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "kubectl $*" >>"${TEST_COMMAND_LOG}"
args=" $* "
if [[ "${args}" == *" config get-contexts "* ]]; then
  printf 'gke_test-project_us-central1-a_agentgateway-benchmark\n'
elif [[ "${args}" == *" get nodes "* ]]; then
  # No GPU nodes makes scale-up time out; zero is immediately ready.
  exit 0
fi
exit 0
EOF

cat >"${TEMP_DIR}/bin/gcloud" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "gcloud $*" >>"${TEST_COMMAND_LOG}"
args=" $* "
format=""
pool=""
previous=""
for arg in "$@"; do
  [[ "${previous}" != "--format" ]] || format="${arg}"
  [[ "${arg}" != --format=* ]] || format="${arg#--format=}"
  [[ "${previous}" != "node-pools" || "${arg}" == "describe" || "${arg}" == "create" ]] || pool="${arg}"
  previous="${arg}"
done
format="${format#value(}"
format="${format%)}"

if [[ "${args}" == *" auth list "* ]]; then
  printf 'tester@example.com\n'
elif [[ "${args}" == *" projects describe "* ]]; then
  printf 'test-project\n'
elif [[ "${args}" == *" projects get-iam-policy "* ]]; then
  [[ "${TEST_MISSING_IAM_ROLE:-false}" != true ]] || exit 0
  for arg in "$@"; do
    if [[ "${arg}" == --filter=bindings.role=* ]]; then
      role="${arg#--filter=bindings.role=}"
      printf '%s\n' "${role%% AND*}"
    fi
  done
elif [[ "${args}" == *" services list "* ]]; then
  for arg in "$@"; do
    [[ "${arg}" == --filter=config.name=* ]] && printf '%s\n' "${arg#--filter=config.name=}"
  done
elif [[ "${args}" == *" compute machine-types describe "* ]]; then
  printf 'a3-highgpu-2g\n'
elif [[ "${args}" == *" compute accelerator-types describe "* ]]; then
  printf 'nvidia-h100-80gb\n'
elif [[ "${args}" == *" iam service-accounts describe "* ]]; then
  if [[ "${format}" == "description" ]]; then
    printf 'agentgateway inference benchmark GKE nodes\n'
  else
    printf 'agentgateway-benchmark-nodes@test-project.iam.gserviceaccount.com\n'
  fi
elif [[ "${args}" == *" container clusters create "* ]]; then
  touch "${TEST_STATE_DIR}/cluster" "${TEST_STATE_DIR}/pd-csi" \
    "${TEST_STATE_DIR}/filestore-csi" "${TEST_STATE_DIR}/node-local-dns" \
    "${TEST_STATE_DIR}/managed-prometheus" "${TEST_STATE_DIR}/monitoring-components"
elif [[ "${args}" == *" container clusters delete "* ]]; then
  rm -f "${TEST_STATE_DIR}/cluster" "${TEST_STATE_DIR}/bench-cpu" \
    "${TEST_STATE_DIR}/gpu-h100"
elif [[ "${args}" == *" container clusters update "* ]]; then
  [[ "${args}" != *"GcePersistentDiskCsiDriver=ENABLED"* ]] || touch "${TEST_STATE_DIR}/pd-csi"
  [[ "${args}" != *"GcpFilestoreCsiDriver=ENABLED"* ]] || touch "${TEST_STATE_DIR}/filestore-csi"
  [[ "${args}" != *"NodeLocalDNS=ENABLED"* ]] || touch "${TEST_STATE_DIR}/node-local-dns"
  [[ "${args}" != *" --enable-managed-prometheus "* ]] || touch "${TEST_STATE_DIR}/managed-prometheus"
  [[ "${args}" != *" --monitoring="* ]] || touch "${TEST_STATE_DIR}/monitoring-components"
elif [[ "${args}" == *" container clusters describe "* ]]; then
  [[ -f "${TEST_STATE_DIR}/cluster" ]] || exit 1
  case "${format}" in
    name) printf 'agentgateway-benchmark\n' ;;
    resourceLabels.managed_by) printf 'agentgateway-benchmark\n' ;;
    resourceLabels.purpose) printf 'inference-benchmark\n' ;;
    network) printf 'default\n' ;;
    subnetwork) printf 'default\n' ;;
    workloadIdentityConfig.workloadPool) printf 'test-project.svc.id.goog\n' ;;
    addonsConfig.gcePersistentDiskCsiDriverConfig.enabled)
      [[ -f "${TEST_STATE_DIR}/pd-csi" ]] && printf 'True\n' || printf 'False\n'
      ;;
    addonsConfig.gcpFilestoreCsiDriverConfig.enabled)
      [[ -f "${TEST_STATE_DIR}/filestore-csi" ]] && printf 'True\n' || printf 'False\n'
      ;;
    addonsConfig.dnsCacheConfig.enabled)
      [[ -f "${TEST_STATE_DIR}/node-local-dns" ]] && printf 'True\n' || printf 'False\n'
      ;;
    monitoringConfig.managedPrometheusConfig.enabled)
      [[ -f "${TEST_STATE_DIR}/managed-prometheus" ]] && printf 'True\n' || printf 'False\n'
      ;;
    monitoringConfig.componentConfig.enableComponents)
      if [[ -f "${TEST_STATE_DIR}/monitoring-components" ]]; then
        printf 'SYSTEM_COMPONENTS;CADVISOR;KUBELET\n'
      else
        printf 'SYSTEM_COMPONENTS\n'
      fi
      ;;
    currentMasterVersion) printf '1.34.0-gke.test\n' ;;
  esac
elif [[ "${args}" == *" container node-pools create "* ]]; then
  if [[ "${args}" == *" bench-cpu "* ]]; then
    touch "${TEST_STATE_DIR}/bench-cpu"
  elif [[ "${args}" == *" gpu-h100 "* ]]; then
    touch "${TEST_STATE_DIR}/gpu-h100"
  fi
elif [[ "${args}" == *" container node-pools describe "* ]]; then
  if [[ "${args}" == *" default-pool "* ]]; then
    pool=default-pool
  elif [[ "${args}" == *" bench-cpu "* ]]; then
    pool=bench-cpu
    [[ -f "${TEST_STATE_DIR}/bench-cpu" ]] || exit 1
  elif [[ "${args}" == *" gpu-h100 "* ]]; then
    pool=gpu-h100
    [[ -f "${TEST_STATE_DIR}/gpu-h100" ]] || exit 1
  else
    exit 1
  fi
  case "${format}" in
    name) printf '%s\n' "${pool}" ;;
    config.machineType)
      [[ "${pool}" == default-pool ]] && printf 'e2-standard-4\n' || \
        { [[ "${pool}" == bench-cpu ]] && printf 'e2-standard-32\n' || printf 'a3-highgpu-2g\n'; }
      ;;
    'locations[0]') printf 'us-central1-a\n' ;;
    config.serviceAccount) printf 'agentgateway-benchmark-nodes@test-project.iam.gserviceaccount.com\n' ;;
    'config.accelerators[0].acceleratorType') printf 'nvidia-h100-80gb\n' ;;
    'config.accelerators[0].acceleratorCount') printf '2\n' ;;
    config.spot) printf 'True\n' ;;
    'instanceGroupUrls[]')
      printf 'https://example/projects/test/zones/us-central1-a/instanceGroupManagers/%s-group\n' "${pool}"
      ;;
  esac
elif [[ "${args}" == *" compute instance-groups managed describe "* ]]; then
  printf '1\n'
fi
exit 0
EOF

chmod +x "${TEMP_DIR}/bin/gcloud" "${TEMP_DIR}/bin/kubectl" \
  "${TEMP_DIR}/bin/gke-gcloud-auth-plugin" "${TEMP_DIR}/bin/sleep"

export PATH="${TEMP_DIR}/bin:/usr/bin:/bin"
export TEST_COMMAND_LOG="${TEMP_DIR}/commands.log"
export TEST_STATE_DIR="${TEMP_DIR}/state"
export BENCHMARK_GKE_PROJECT=test-project
export BENCHMARK_GKE_GPU_READY_TIMEOUT=1
export BENCHMARK_GKE_PROVISIONING_MANIFEST="${TEMP_DIR}/provisioning.yaml"

: >"${TEST_COMMAND_LOG}"
"${PROVISION_DIR}/provision.sh" plan >"${TEMP_DIR}/plan.log"
grep -q 'GPU target: 8 x a3-highgpu-2g' "${TEMP_DIR}/plan.log"
grep -q 'accelerators: 2 x nvidia-h100-80gb per node' "${TEMP_DIR}/plan.log"

BENCHMARK_GKE_GPU_MACHINE_TYPE=custom-gpu-machine \
BENCHMARK_GKE_GPU_ACCELERATOR_TYPE=custom-gpu \
BENCHMARK_GKE_GPU_ACCELERATORS_PER_NODE=4 \
BENCHMARK_GKE_GPU_TARGET_NODES=3 \
  "${PROVISION_DIR}/provision.sh" plan >"${TEMP_DIR}/custom-plan.log"
grep -q 'GPU target: 3 x custom-gpu-machine' "${TEMP_DIR}/custom-plan.log"
grep -q 'accelerators: 4 x custom-gpu per node' "${TEMP_DIR}/custom-plan.log"

: >"${TEST_COMMAND_LOG}"
if TEST_MISSING_IAM_ROLE=true "${PROVISION_DIR}/provision.sh" plan \
  >"${TEMP_DIR}/missing-role.log" 2>&1; then
  echo "planning unexpectedly accepted a node account without its project roles" >&2
  exit 1
fi
grep -q 'the provisioner does not modify project IAM' "${TEMP_DIR}/missing-role.log"
if grep -Eq 'service-accounts create|add-iam-policy-binding' "${TEST_COMMAND_LOG}"; then
  echo "IAM validation unexpectedly mutated project IAM" >&2
  exit 1
fi

: >"${TEST_COMMAND_LOG}"
BENCHMARK_GKE_NODE_SERVICE_ACCOUNT=default \
  "${PROVISION_DIR}/provision.sh" plan >"${TEMP_DIR}/default-service-account-plan.log"
grep -q 'WARNING: using the Compute Engine default service account' \
  "${TEMP_DIR}/default-service-account-plan.log"
if grep -q 'iam service-accounts describe' "${TEST_COMMAND_LOG}"; then
  echo "default service-account mode unexpectedly queried a dedicated account" >&2
  exit 1
fi

: >"${TEST_COMMAND_LOG}"
if ! "${PROVISION_DIR}/provision.sh" apply >"${TEMP_DIR}/apply.log" 2>&1; then
  cat "${TEMP_DIR}/apply.log" >&2
  cat "${TEST_COMMAND_LOG}" >&2
  exit 1
fi
grep -q -- '--machine-type a3-highgpu-2g --num-nodes 0' "${TEST_COMMAND_LOG}"
grep -q -- '--accelerator=type=nvidia-h100-80gb,count=2,gpu-driver-version=default' "${TEST_COMMAND_LOG}"
grep -q -- '--spot --quiet' "${TEST_COMMAND_LOG}"
if grep -q 'add-iam-policy-binding' "${TEST_COMMAND_LOG}"; then
  echo "cluster provisioning unexpectedly modified project IAM" >&2
  exit 1
fi

rm -f "${TEST_STATE_DIR}/pd-csi" "${TEST_STATE_DIR}/filestore-csi" \
  "${TEST_STATE_DIR}/node-local-dns" "${TEST_STATE_DIR}/managed-prometheus" \
  "${TEST_STATE_DIR}/monitoring-components"
: >"${TEST_COMMAND_LOG}"
"${PROVISION_DIR}/provision.sh" apply >"${TEMP_DIR}/reconcile.log"
grep -q -- '--update-addons=GcePersistentDiskCsiDriver=ENABLED' "${TEST_COMMAND_LOG}"
grep -q -- '--update-addons=GcpFilestoreCsiDriver=ENABLED' "${TEST_COMMAND_LOG}"
grep -q -- '--update-addons=NodeLocalDNS=ENABLED' "${TEST_COMMAND_LOG}"
grep -q -- '--enable-managed-prometheus --quiet' "${TEST_COMMAND_LOG}"
grep -q -- '--monitoring=SYSTEM,CADVISOR,KUBELET --quiet' "${TEST_COMMAND_LOG}"
if awk '/container clusters update/ && /--update-addons/ &&
    (/--enable-managed-prometheus/ || /--monitoring=/) { found=1 } END { exit !found }' \
    "${TEST_COMMAND_LOG}"; then
  echo "add-on and monitoring reconciliation were combined into one GKE update" >&2
  exit 1
fi

: >"${TEST_COMMAND_LOG}"
if "${PROVISION_DIR}/scale-gpu.sh" up >"${TEMP_DIR}/scale.log" 2>&1; then
  echo "GPU scale-up unexpectedly succeeded without nodes" >&2
  exit 1
fi
grep -q -- '--num-nodes 8' "${TEST_COMMAND_LOG}"
grep -q -- '--num-nodes 0' "${TEST_COMMAND_LOG}"
grep -q 'rolling GPU pool gpu-h100 back to zero' "${TEMP_DIR}/scale.log"

: >"${TEST_COMMAND_LOG}"
if "${PROVISION_DIR}/destroy.sh" >"${TEMP_DIR}/destroy-guard.log" 2>&1; then
  echo "cluster destruction unexpectedly succeeded without its explicit guard" >&2
  exit 1
fi
grep -q 'BENCHMARK_GKE_ALLOW_DESTROY=true' "${TEMP_DIR}/destroy-guard.log"
if grep -q 'container clusters delete' "${TEST_COMMAND_LOG}"; then
  echo "guarded destruction reached the cluster delete command" >&2
  exit 1
fi

: >"${TEST_COMMAND_LOG}"
BENCHMARK_GKE_ALLOW_DESTROY=true "${PROVISION_DIR}/destroy.sh" \
  >"${TEMP_DIR}/destroy.log"
grep -q 'container clusters delete agentgateway-benchmark' "${TEST_COMMAND_LOG}"
[[ ! -f "${TEST_STATE_DIR}/cluster" ]]

: >"${TEST_COMMAND_LOG}"
BENCHMARK_GKE_ALLOW_DESTROY=true BENCHMARK_GKE_DESTROY_IF_MISSING=true \
  "${PROVISION_DIR}/destroy.sh" >"${TEMP_DIR}/destroy-missing.log"
grep -q 'cluster agentgateway-benchmark is already absent' \
  "${TEMP_DIR}/destroy-missing.log"
if grep -q 'container clusters delete' "${TEST_COMMAND_LOG}"; then
  echo "idempotent destruction attempted to delete an absent cluster" >&2
  exit 1
fi

touch "${TEST_STATE_DIR}/cluster"
: >"${TEST_COMMAND_LOG}"
BENCHMARK_GKE_ALLOW_DESTROY=true "${PROVISION_DIR}/destroy.sh" \
  >"${TEMP_DIR}/destroy-partial.log"
grep -q 'GPU node pool gpu-h100 is already absent' \
  "${TEMP_DIR}/destroy-partial.log"
grep -q 'container clusters delete agentgateway-benchmark' "${TEST_COMMAND_LOG}"
[[ ! -f "${TEST_STATE_DIR}/cluster" ]]

echo "GKE provisioning tests passed"
