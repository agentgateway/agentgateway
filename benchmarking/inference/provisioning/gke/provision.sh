#!/usr/bin/env bash
# Create or verify the GKE infrastructure used by inference benchmark campaigns.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

usage() {
  cat <<'EOF'
Usage: provision.sh plan|apply

  plan   Validate local prerequisites and show the desired GKE profile.
  apply  Idempotently create or verify the cluster and its node pools.
EOF
}

api_enabled() {
  gcloud services list --enabled --project "${GKE_PROJECT}" \
    --filter="config.name=$1" --format='value(config.name)' | grep -Fxq "$1"
}

validate_hardware_catalog() {
  gcloud compute machine-types describe "${GKE_GPU_MACHINE_TYPE}" \
    --project "${GKE_PROJECT}" --zone "${GKE_LOCATION}" \
    --format='value(name)' >/dev/null || \
    die "machine type ${GKE_GPU_MACHINE_TYPE} is unavailable in ${GKE_LOCATION}"
  gcloud compute accelerator-types describe "${GKE_GPU_ACCELERATOR_TYPE}" \
    --project "${GKE_PROJECT}" --zone "${GKE_LOCATION}" \
    --format='value(name)' >/dev/null || \
    die "accelerator ${GKE_GPU_ACCELERATOR_TYPE} is unavailable in ${GKE_LOCATION}"
}

ensure_apis() {
  local api
  local -a apis=(
    container.googleapis.com
    compute.googleapis.com
    file.googleapis.com
    iam.googleapis.com
    monitoring.googleapis.com
  )
  for api in "${apis[@]}"; do
    if ! api_enabled "${api}"; then
      log "enabling ${api}"
      gcloud services enable "${api}" --project "${GKE_PROJECT}" --quiet
    fi
  done
}

node_service_account_has_role() {
  local role="$1" binding
  binding="$(gcloud projects get-iam-policy "${GKE_PROJECT}" \
    --flatten='bindings[].members' \
    --filter="bindings.role=${role} AND bindings.members=serviceAccount:${GKE_NODE_SERVICE_ACCOUNT}" \
    --format='value(bindings.role)')"
  [[ "${binding}" == "${role}" ]]
}

validate_node_service_account() {
  if [[ "${GKE_NODE_SERVICE_ACCOUNT}" == default ]]; then
    log "WARNING: using the Compute Engine default service account; prefer a dedicated minimally privileged account"
    return
  fi
  gcloud iam service-accounts describe "${GKE_NODE_SERVICE_ACCOUNT}" \
    --project "${GKE_PROJECT}" --format='value(email)' >/dev/null 2>&1 || \
    die "node service account ${GKE_NODE_SERVICE_ACCOUNT} does not exist; an IAM administrator must create it"

  local role missing=0
  for role in roles/container.defaultNodeServiceAccount roles/artifactregistry.reader; do
    if ! node_service_account_has_role "${role}"; then
      log "missing node service-account role: ${role}" >&2
      missing=1
    fi
  done
  if (( missing != 0 )); then
    die "an IAM administrator must grant the roles above to serviceAccount:${GKE_NODE_SERVICE_ACCOUNT}; the provisioner does not modify project IAM"
  fi
}

create_cluster() {
  [[ "${GKE_SYSTEM_NODEPOOL}" == default-pool ]] || \
    die "new clusters currently require BENCHMARK_GKE_SYSTEM_NODEPOOL=default-pool"
  log "creating zonal GKE cluster ${GKE_PROJECT}/${GKE_LOCATION}/${GKE_CLUSTER}"
  gcloud container clusters create "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --node-locations "${GKE_LOCATION}" \
    --release-channel "${GKE_RELEASE_CHANNEL}" \
    --network "${GKE_NETWORK}" --subnetwork "${GKE_SUBNETWORK}" \
    --enable-ip-alias \
    --workload-pool "${GKE_PROJECT}.svc.id.goog" \
    --addons=GcePersistentDiskCsiDriver,GcpFilestoreCsiDriver,NodeLocalDNS \
    --enable-managed-prometheus --monitoring=SYSTEM,CADVISOR,KUBELET \
    --logging=SYSTEM,WORKLOAD \
    --labels="managed_by=${MANAGED_BY},purpose=inference-benchmark" \
    --machine-type "${GKE_SYSTEM_MACHINE_TYPE}" \
    --num-nodes "${GKE_SYSTEM_NODES}" \
    --disk-type pd-balanced --disk-size 100 \
    --image-type COS_CONTAINERD \
    --service-account "${GKE_NODE_SERVICE_ACCOUNT}" \
    --scopes=https://www.googleapis.com/auth/cloud-platform \
    --node-labels=benchmark.agentgateway.dev/role=system \
    --quiet
}

reconcile_cluster_addons() {
  log "ensuring storage, DNS, and monitoring add-ons are enabled"
  # `gcloud container clusters update` accepts exactly one mutation family.
  # Keep add-on, managed Prometheus, and monitoring-component reconciliation
  # in separate calls rather than combining their otherwise valid flags.
  local addon field enabled
  while IFS=$'\t' read -r addon field; do
    enabled="$(gcloud container clusters describe "${GKE_CLUSTER}" \
      --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
      --format="value(${field})")"
    if [[ "${enabled}" != True && "${enabled}" != true ]]; then
      log "enabling GKE add-on ${addon}"
      gcloud container clusters update "${GKE_CLUSTER}" \
        --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
        --update-addons="${addon}=ENABLED" --quiet
    fi
  done <<'EOF'
GcePersistentDiskCsiDriver	addonsConfig.gcePersistentDiskCsiDriverConfig.enabled
GcpFilestoreCsiDriver	addonsConfig.gcpFilestoreCsiDriverConfig.enabled
NodeLocalDNS	addonsConfig.dnsCacheConfig.enabled
EOF

  enabled="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(monitoringConfig.managedPrometheusConfig.enabled)')"
  if [[ "${enabled}" != True && "${enabled}" != true ]]; then
    log "enabling Google Managed Service for Prometheus"
    gcloud container clusters update "${GKE_CLUSTER}" \
      --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
      --enable-managed-prometheus --quiet
  fi

  local components component normalized csv=""
  components="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(monitoringConfig.componentConfig.enableComponents)')"
  if [[ ";${components};" != *';SYSTEM_COMPONENTS;'* || \
        ";${components};" != *';CADVISOR;'* || \
        ";${components};" != *';KUBELET;'* ]]; then
    IFS=';' read -r -a current_components <<<"${components}"
    for component in "${current_components[@]}" SYSTEM CADVISOR KUBELET; do
      [[ -n "${component}" ]] || continue
      [[ "${component}" != SYSTEM_COMPONENTS ]] || component=SYSTEM
      normalized=",${csv},"
      [[ "${normalized}" != *",${component},"* ]] || continue
      [[ -z "${csv}" ]] || csv+=,
      csv+="${component}"
    done
    log "enabling required GKE monitoring components"
    gcloud container clusters update "${GKE_CLUSTER}" \
      --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
      --monitoring="${csv}" --quiet
  fi
}

create_harness_pool() {
  log "creating CPU harness node pool ${GKE_HARNESS_NODEPOOL}"
  gcloud container node-pools create "${GKE_HARNESS_NODEPOOL}" \
    --cluster "${GKE_CLUSTER}" --project "${GKE_PROJECT}" \
    --location "${GKE_LOCATION}" --node-locations "${GKE_LOCATION}" \
    --machine-type "${GKE_HARNESS_MACHINE_TYPE}" \
    --num-nodes "${GKE_HARNESS_NODES}" \
    --disk-type pd-balanced --disk-size 100 \
    --image-type COS_CONTAINERD \
    --service-account "${GKE_NODE_SERVICE_ACCOUNT}" \
    --scopes=https://www.googleapis.com/auth/cloud-platform \
    --node-labels=benchmark.agentgateway.dev/role=harness \
    --enable-autorepair --enable-autoupgrade --quiet
}

create_gpu_pool() {
  local -a spot_args=()
  [[ "${GKE_GPU_SPOT}" == false ]] || spot_args+=(--spot)
  log "creating zero-sized GPU node pool ${GKE_GPU_NODEPOOL}"
  gcloud container node-pools create "${GKE_GPU_NODEPOOL}" \
    --cluster "${GKE_CLUSTER}" --project "${GKE_PROJECT}" \
    --location "${GKE_LOCATION}" --node-locations "${GKE_LOCATION}" \
    --machine-type "${GKE_GPU_MACHINE_TYPE}" --num-nodes 0 \
    --accelerator="type=${GKE_GPU_ACCELERATOR_TYPE},count=${GKE_GPU_ACCELERATORS_PER_NODE},gpu-driver-version=${GKE_GPU_DRIVER_VERSION}" \
    --disk-type pd-ssd --disk-size 200 \
    --image-type COS_CONTAINERD \
    --service-account "${GKE_NODE_SERVICE_ACCOUNT}" \
    --scopes=https://www.googleapis.com/auth/cloud-platform \
    --node-labels=benchmark.agentgateway.dev/role=model-server \
    --enable-autorepair --enable-autoupgrade "${spot_args[@]}" --quiet
}

plan() {
  print_config
  local api
  for api in container.googleapis.com compute.googleapis.com file.googleapis.com \
    iam.googleapis.com monitoring.googleapis.com; do
    if api_enabled "${api}"; then
      log "${api}: enabled"
    else
      log "${api}: will enable"
    fi
  done
  if cluster_exists; then
    assert_managed_cluster
    log "cluster: exists and is managed by ${MANAGED_BY}"
  else
    log "cluster: will create"
  fi
  validate_node_service_account
  if api_enabled compute.googleapis.com; then
    validate_hardware_catalog
  fi
}

apply() {
  ensure_apis
  validate_hardware_catalog
  validate_node_service_account
  if cluster_exists; then
    assert_managed_cluster
    log "cluster ${GKE_CLUSTER} already exists"
  else
    create_cluster
  fi
  reconcile_cluster_addons
  get_credentials

  verify_cpu_pool "${GKE_SYSTEM_NODEPOOL}" "${GKE_SYSTEM_MACHINE_TYPE}"
  if node_pool_exists "${GKE_HARNESS_NODEPOOL}"; then
    verify_cpu_pool "${GKE_HARNESS_NODEPOOL}" "${GKE_HARNESS_MACHINE_TYPE}"
  else
    create_harness_pool
  fi
  if node_pool_exists "${GKE_GPU_NODEPOOL}"; then
    verify_gpu_pool
  else
    create_gpu_pool
  fi
  "${SCRIPT_DIR}/verify.sh"
}

main() {
  [[ $# -eq 1 ]] || { usage >&2; exit 2; }
  if [[ "$1" == -h || "$1" == --help ]]; then
    usage
    return
  fi
  export CLOUDSDK_CORE_DISABLE_PROMPTS=1
  preflight
  case "$1" in
    plan) plan ;;
    apply) apply ;;
    *) usage >&2; exit 2 ;;
  esac
}

main "$@"
