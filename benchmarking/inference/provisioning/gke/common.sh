#!/usr/bin/env bash

# Shared validation and identity helpers for the GKE provisioning commands.
# Variables are consumed by the scripts that source this file.
# shellcheck disable=SC2034

MANAGED_BY="agentgateway-benchmark"

log() { echo "[gke-provision] $*"; }
die() { log "ERROR: $*" >&2; exit 1; }

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required but was not found in PATH"
}

is_positive_integer() {
  [[ "$1" =~ ^[1-9][0-9]*$ ]]
}

is_nonnegative_integer() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

validate_bool() {
  case "$2" in
    true|false) ;;
    *) die "$1 must be true or false" ;;
  esac
}

load_config() {
  GKE_PROJECT="${BENCHMARK_GKE_PROJECT:-}"
  GKE_LOCATION="${BENCHMARK_GKE_LOCATION:-us-central1-a}"
  GKE_CLUSTER="${BENCHMARK_GKE_CLUSTER:-agentgateway-benchmark}"
  GKE_NETWORK="${BENCHMARK_GKE_NETWORK:-default}"
  GKE_SUBNETWORK="${BENCHMARK_GKE_SUBNETWORK:-default}"
  GKE_RELEASE_CHANNEL="${BENCHMARK_GKE_RELEASE_CHANNEL:-stable}"

  GKE_SYSTEM_NODEPOOL="${BENCHMARK_GKE_SYSTEM_NODEPOOL:-default-pool}"
  GKE_SYSTEM_MACHINE_TYPE="${BENCHMARK_GKE_SYSTEM_MACHINE_TYPE:-e2-standard-4}"
  GKE_SYSTEM_NODES="${BENCHMARK_GKE_SYSTEM_NODES:-1}"
  GKE_HARNESS_NODEPOOL="${BENCHMARK_GKE_HARNESS_NODEPOOL:-bench-cpu}"
  GKE_HARNESS_MACHINE_TYPE="${BENCHMARK_GKE_HARNESS_MACHINE_TYPE:-e2-standard-32}"
  GKE_HARNESS_NODES="${BENCHMARK_GKE_HARNESS_NODES:-1}"

  GKE_GPU_NODEPOOL="${BENCHMARK_GKE_GPU_NODEPOOL:-gpu-h100}"
  GKE_GPU_MACHINE_TYPE="${BENCHMARK_GKE_GPU_MACHINE_TYPE:-a3-highgpu-2g}"
  GKE_GPU_ACCELERATOR_TYPE="${BENCHMARK_GKE_GPU_ACCELERATOR_TYPE:-nvidia-h100-80gb}"
  GKE_GPU_ACCELERATORS_PER_NODE="${BENCHMARK_GKE_GPU_ACCELERATORS_PER_NODE:-2}"
  GKE_GPU_TARGET_NODES="${BENCHMARK_GKE_GPU_TARGET_NODES:-8}"
  GKE_GPU_DRIVER_VERSION="${BENCHMARK_GKE_GPU_DRIVER_VERSION:-default}"
  GKE_GPU_SPOT="${BENCHMARK_GKE_GPU_SPOT:-true}"
  GKE_GPU_READY_TIMEOUT="${BENCHMARK_GKE_GPU_READY_TIMEOUT:-2400}"

  GKE_NODE_SERVICE_ACCOUNT_ID="${BENCHMARK_GKE_NODE_SERVICE_ACCOUNT_ID:-agentgateway-benchmark-nodes}"
  GKE_NODE_SERVICE_ACCOUNT="${BENCHMARK_GKE_NODE_SERVICE_ACCOUNT:-${GKE_NODE_SERVICE_ACCOUNT_ID}@${GKE_PROJECT}.iam.gserviceaccount.com}"
  GKE_KUBE_CONTEXT="gke_${GKE_PROJECT}_${GKE_LOCATION}_${GKE_CLUSTER}"
}

validate_config() {
  [[ -n "${GKE_PROJECT}" ]] || die "BENCHMARK_GKE_PROJECT is required"
  [[ "${GKE_LOCATION}" =~ ^[a-z]+-[a-z0-9]+[0-9]-[a-z]$ ]] || \
    die "BENCHMARK_GKE_LOCATION must be a zone such as us-central1-a"
  [[ "${GKE_CLUSTER}" =~ ^[a-z][a-z0-9-]{0,38}[a-z0-9]$ ]] || \
    die "BENCHMARK_GKE_CLUSTER is not a valid GKE cluster name"
  [[ "${GKE_SYSTEM_NODEPOOL}" =~ ^[a-z][a-z0-9-]{0,38}[a-z0-9]$ ]] || \
    die "BENCHMARK_GKE_SYSTEM_NODEPOOL is not a valid node-pool name"
  [[ "${GKE_HARNESS_NODEPOOL}" =~ ^[a-z][a-z0-9-]{0,38}[a-z0-9]$ ]] || \
    die "BENCHMARK_GKE_HARNESS_NODEPOOL is not a valid node-pool name"
  [[ "${GKE_GPU_NODEPOOL}" =~ ^[a-z][a-z0-9-]{0,38}[a-z0-9]$ ]] || \
    die "BENCHMARK_GKE_GPU_NODEPOOL is not a valid node-pool name"
  [[ "${GKE_SYSTEM_NODEPOOL}" != "${GKE_HARNESS_NODEPOOL}" && \
     "${GKE_SYSTEM_NODEPOOL}" != "${GKE_GPU_NODEPOOL}" && \
     "${GKE_HARNESS_NODEPOOL}" != "${GKE_GPU_NODEPOOL}" ]] || \
    die "GKE node-pool names must be distinct"
  is_positive_integer "${GKE_SYSTEM_NODES}" || \
    die "BENCHMARK_GKE_SYSTEM_NODES must be a positive integer"
  is_positive_integer "${GKE_HARNESS_NODES}" || \
    die "BENCHMARK_GKE_HARNESS_NODES must be a positive integer"
  is_positive_integer "${GKE_GPU_ACCELERATORS_PER_NODE}" || \
    die "BENCHMARK_GKE_GPU_ACCELERATORS_PER_NODE must be a positive integer"
  is_positive_integer "${GKE_GPU_TARGET_NODES}" || \
    die "BENCHMARK_GKE_GPU_TARGET_NODES must be a positive integer"
  is_positive_integer "${GKE_GPU_READY_TIMEOUT}" || \
    die "BENCHMARK_GKE_GPU_READY_TIMEOUT must be a positive integer"
  validate_bool BENCHMARK_GKE_GPU_SPOT "${GKE_GPU_SPOT}"
  if [[ "${GKE_NODE_SERVICE_ACCOUNT}" != default ]]; then
    [[ "${GKE_NODE_SERVICE_ACCOUNT}" == *@"${GKE_PROJECT}".iam.gserviceaccount.com ]] || \
      die "BENCHMARK_GKE_NODE_SERVICE_ACCOUNT must be 'default' or belong to ${GKE_PROJECT}"
  fi
}

preflight() {
  require_command gcloud
  require_command kubectl
  require_command gke-gcloud-auth-plugin
  load_config
  validate_config

  local account
  account="$(gcloud auth list --filter=status:ACTIVE --limit=1 --format='value(account)')"
  [[ -n "${account}" ]] || die "gcloud has no active authenticated account"
  gcloud projects describe "${GKE_PROJECT}" --format='value(projectId)' >/dev/null || \
    die "gcloud cannot access project ${GKE_PROJECT}"
}

cluster_exists() {
  gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(name)' >/dev/null 2>&1
}

node_pool_exists() {
  gcloud container node-pools describe "$1" --cluster "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(name)' >/dev/null 2>&1
}

assert_managed_cluster() {
  local name managed_by purpose
  name="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(name)')"
  managed_by="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(resourceLabels.managed_by)')"
  purpose="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(resourceLabels.purpose)')"
  [[ "${name}" == "${GKE_CLUSTER}" ]] || die "could not validate cluster identity"
  [[ "${managed_by}" == "${MANAGED_BY}" && "${purpose}" == "inference-benchmark" ]] || \
    die "refusing to manage cluster ${GKE_CLUSTER}: ownership labels do not match"
}

get_credentials() {
  gcloud container clusters get-credentials "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" --quiet >/dev/null
  kubectl config get-contexts "${GKE_KUBE_CONTEXT}" -o name | \
    grep -Fxq "${GKE_KUBE_CONTEXT}" || die "expected kube context ${GKE_KUBE_CONTEXT} was not created"
}

pool_value() {
  local pool="$1" field="$2"
  gcloud container node-pools describe "${pool}" --cluster "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format="value(${field})"
}

verify_cpu_pool() {
  local pool="$1" expected_machine="$2" actual_machine actual_location service_account
  node_pool_exists "${pool}" || die "node pool ${pool} does not exist"
  actual_machine="$(pool_value "${pool}" 'config.machineType')"
  actual_location="$(pool_value "${pool}" 'locations[0]')"
  service_account="$(pool_value "${pool}" 'config.serviceAccount')"
  [[ "${actual_machine}" == "${expected_machine}" ]] || \
    die "node pool ${pool} uses ${actual_machine}; expected ${expected_machine}"
  [[ "${actual_location}" == "${GKE_LOCATION}" ]] || \
    die "node pool ${pool} is in ${actual_location}; expected ${GKE_LOCATION}"
  [[ "${service_account}" == "${GKE_NODE_SERVICE_ACCOUNT}" ]] || \
    die "node pool ${pool} uses service account ${service_account}; expected ${GKE_NODE_SERVICE_ACCOUNT}"
}

verify_gpu_pool() {
  local machine accelerator count spot location service_account
  node_pool_exists "${GKE_GPU_NODEPOOL}" || die "node pool ${GKE_GPU_NODEPOOL} does not exist"
  machine="$(pool_value "${GKE_GPU_NODEPOOL}" 'config.machineType')"
  accelerator="$(pool_value "${GKE_GPU_NODEPOOL}" 'config.accelerators[0].acceleratorType')"
  count="$(pool_value "${GKE_GPU_NODEPOOL}" 'config.accelerators[0].acceleratorCount')"
  spot="$(pool_value "${GKE_GPU_NODEPOOL}" 'config.spot')"
  location="$(pool_value "${GKE_GPU_NODEPOOL}" 'locations[0]')"
  service_account="$(pool_value "${GKE_GPU_NODEPOOL}" 'config.serviceAccount')"
  [[ "${machine}" == "${GKE_GPU_MACHINE_TYPE}" ]] || \
    die "GPU pool uses ${machine}; expected ${GKE_GPU_MACHINE_TYPE}"
  [[ "${accelerator}" == "${GKE_GPU_ACCELERATOR_TYPE}" ]] || \
    die "GPU pool uses ${accelerator}; expected ${GKE_GPU_ACCELERATOR_TYPE}"
  [[ "${count}" == "${GKE_GPU_ACCELERATORS_PER_NODE}" ]] || \
    die "GPU pool has ${count} accelerator(s) per node; expected ${GKE_GPU_ACCELERATORS_PER_NODE}"
  [[ "${location}" == "${GKE_LOCATION}" ]] || \
    die "GPU pool is in ${location}; expected ${GKE_LOCATION}"
  [[ "${service_account}" == "${GKE_NODE_SERVICE_ACCOUNT}" ]] || \
    die "GPU pool uses service account ${service_account}; expected ${GKE_NODE_SERVICE_ACCOUNT}"
  if [[ "${GKE_GPU_SPOT}" == true ]]; then
    [[ "${spot}" == "True" || "${spot}" == "true" ]] || die "GPU pool is not configured for Spot VMs"
  else
    [[ -z "${spot}" || "${spot}" == "False" || "${spot}" == "false" ]] || \
      die "GPU pool unexpectedly uses Spot VMs"
  fi
}

print_config() {
  cat <<EOF
project: ${GKE_PROJECT}
location: ${GKE_LOCATION}
cluster: ${GKE_CLUSTER}
network: ${GKE_NETWORK}
subnetwork: ${GKE_SUBNETWORK}
node service account: ${GKE_NODE_SERVICE_ACCOUNT}
system pool: ${GKE_SYSTEM_NODEPOOL} (${GKE_SYSTEM_NODES} x ${GKE_SYSTEM_MACHINE_TYPE})
harness pool: ${GKE_HARNESS_NODEPOOL} (${GKE_HARNESS_NODES} x ${GKE_HARNESS_MACHINE_TYPE})
GPU pool: ${GKE_GPU_NODEPOOL} (initially zero)
GPU target: ${GKE_GPU_TARGET_NODES} x ${GKE_GPU_MACHINE_TYPE}
accelerators: ${GKE_GPU_ACCELERATORS_PER_NODE} x ${GKE_GPU_ACCELERATOR_TYPE} per node
Spot: ${GKE_GPU_SPOT}
kube context: ${GKE_KUBE_CONTEXT}
EOF
}

print_benchmark_exports() {
  cat <<EOF
export BENCHMARK_CLUSTER_PROVIDER=gke
export BENCHMARK_KUBE_CONTEXT=${GKE_KUBE_CONTEXT}
export BENCHMARK_GKE_PROJECT=${GKE_PROJECT}
export BENCHMARK_GKE_LOCATION=${GKE_LOCATION}
export BENCHMARK_GKE_CLUSTER=${GKE_CLUSTER}
export BENCHMARK_GKE_CPU_NODEPOOL=${GKE_SYSTEM_NODEPOOL}
export BENCHMARK_GKE_HARNESS_NODEPOOL=${GKE_HARNESS_NODEPOOL}
export BENCHMARK_GKE_GPU_NODEPOOL=${GKE_GPU_NODEPOOL}
export BENCHMARK_GKE_GPU_MACHINE_TYPE=${GKE_GPU_MACHINE_TYPE}
export BENCHMARK_GKE_GPU_ACCELERATOR_TYPE=${GKE_GPU_ACCELERATOR_TYPE}
export BENCHMARK_GKE_GPU_ACCELERATORS_PER_NODE=${GKE_GPU_ACCELERATORS_PER_NODE}
export BENCHMARK_GKE_GPU_TARGET_NODES=${GKE_GPU_TARGET_NODES}
EOF
}
