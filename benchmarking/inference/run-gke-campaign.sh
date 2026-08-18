#!/usr/bin/env bash
# Run the standard optimized-baseline GKE campaign and generate both reports.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CONTROLLER_DIR="${REPO_ROOT}/controller"
MAKE_BIN="${BENCHMARK_MAKE_BIN:-make}"
FINALIZER_ARMED=false
CAMPAIGN_CLEANUP_REQUIRED=false

log() { echo "[gke-campaign] $*"; }
die() { log "ERROR: $*" >&2; exit 1; }

run_make() {
  "${MAKE_BIN}" -C "${CONTROLLER_DIR}" "$@"
}

set_defaults() {
  : "${BENCHMARK_GKE_PROJECT:?BENCHMARK_GKE_PROJECT is required}"
  : "${BENCHMARK_GKE_LOCATION:=us-central1-a}"
  : "${BENCHMARK_GKE_CLUSTER:=agentgateway-benchmark}"
  : "${BENCHMARK_KUBE_CONTEXT:=gke_${BENCHMARK_GKE_PROJECT}_${BENCHMARK_GKE_LOCATION}_${BENCHMARK_GKE_CLUSTER}}"
  : "${BENCHMARK_GKE_CPU_NODEPOOL:=default-pool}"
  : "${BENCHMARK_GKE_HARNESS_NODEPOOL:=bench-cpu}"
  : "${BENCHMARK_GKE_GPU_NODEPOOL:=gpu-h100}"
  : "${BENCHMARK_GKE_GPU_MACHINE_TYPE:=a3-highgpu-2g}"
  : "${BENCHMARK_GKE_GPU_ACCELERATOR_TYPE:=nvidia-h100-80gb}"
  : "${BENCHMARK_GKE_GPU_ACCELERATORS_PER_NODE:=2}"
  : "${BENCHMARK_GKE_GPU_TARGET_NODES:=8}"
  : "${BENCHMARK_GKE_CLUSTER_LIFECYCLE:=retain}"

  : "${BENCHMARK_SUITE:=llm-d-benchmark}"
  : "${BENCHMARK_ACCELERATOR_TYPE:=gpu}"
  : "${BENCHMARK_ACCELERATOR_MODEL:=h100}"
  : "${BENCHMARK_BACKEND_TYPE:=vllm}"
  : "${BENCHMARK_SCENARIO:=optimized-baseline}"
  : "${BENCHMARK_ROUTING_POLICY:=optimized-baseline}"
  : "${BENCHMARK_REFERENCE_PROFILE:=optimized-baseline-qwen3-32b-h100-v0.9}"
  : "${BENCHMARK_WORKLOAD_VARIANT:=upstream}"
  : "${BENCHMARK_REPLICAS:=8}"
  : "${BENCHMARK_TENSOR_PARALLELISM:=2}"
  : "${BENCHMARK_ENDPOINT_PATH:=internal}"
  : "${BENCHMARK_MODEL_STORAGE_PROFILE:=high-throughput-shared}"
  : "${BENCHMARK_WORKLOAD_STORAGE_PROFILE:=shared}"
  : "${BENCHMARK_GPU_RELEASE_POLICY:=after-load}"
  : "${BENCHMARK_REPETITION:=1}"
  : "${BENCHMARK_CAMPAIGN_ID:=optimized-baseline-qwen3-32b-h100-$(date -u +%Y%m%d-%H%M%S)}"
  : "${BENCHMARK_SECRET_NAMESPACE:=benchmark-secrets}"
  : "${BENCHMARK_HF_SECRET_NAME:=llm-d-hf-token}"
  : "${BENCHMARK_REPORT_FORMATS:=markdown,png,csv}"

  export BENCHMARK_GKE_PROJECT BENCHMARK_GKE_LOCATION BENCHMARK_GKE_CLUSTER
  export BENCHMARK_KUBE_CONTEXT BENCHMARK_GKE_CPU_NODEPOOL
  export BENCHMARK_GKE_HARNESS_NODEPOOL BENCHMARK_GKE_GPU_NODEPOOL
  export BENCHMARK_GKE_GPU_MACHINE_TYPE BENCHMARK_GKE_GPU_ACCELERATOR_TYPE
  export BENCHMARK_GKE_GPU_ACCELERATORS_PER_NODE BENCHMARK_GKE_GPU_TARGET_NODES
  export BENCHMARK_GKE_CLUSTER_LIFECYCLE
  export BENCHMARK_SUITE BENCHMARK_ACCELERATOR_TYPE BENCHMARK_ACCELERATOR_MODEL
  export BENCHMARK_BACKEND_TYPE BENCHMARK_SCENARIO BENCHMARK_ROUTING_POLICY
  export BENCHMARK_REFERENCE_PROFILE BENCHMARK_WORKLOAD_VARIANT
  export BENCHMARK_REPLICAS BENCHMARK_TENSOR_PARALLELISM BENCHMARK_ENDPOINT_PATH
  export BENCHMARK_MODEL_STORAGE_PROFILE BENCHMARK_WORKLOAD_STORAGE_PROFILE
  export BENCHMARK_GPU_RELEASE_POLICY BENCHMARK_REPETITION BENCHMARK_CAMPAIGN_ID
  export BENCHMARK_SECRET_NAMESPACE BENCHMARK_HF_SECRET_NAME
  export BENCHMARK_REPORT_FORMATS BENCHMARK_CLUSTER_PROVIDER=gke

  case "${BENCHMARK_GKE_CLUSTER_LIFECYCLE}" in
    retain|destroy) ;;
    *) die "BENCHMARK_GKE_CLUSTER_LIFECYCLE must be retain or destroy" ;;
  esac
}

ensure_hf_secret() {
  kubectl --context "${BENCHMARK_KUBE_CONTEXT}" \
    create namespace "${BENCHMARK_SECRET_NAMESPACE}" \
    --dry-run=client -o yaml |
    kubectl --context "${BENCHMARK_KUBE_CONTEXT}" apply -f - >/dev/null

  if kubectl --context "${BENCHMARK_KUBE_CONTEXT}" \
      --namespace "${BENCHMARK_SECRET_NAMESPACE}" \
      get secret "${BENCHMARK_HF_SECRET_NAME}" >/dev/null 2>&1; then
    local token
    token="$(kubectl --context "${BENCHMARK_KUBE_CONTEXT}" \
      --namespace "${BENCHMARK_SECRET_NAMESPACE}" \
      get secret "${BENCHMARK_HF_SECRET_NAME}" \
      --template='{{index .data "HF_TOKEN" | base64decode}}')"
    [[ "${token}" == hf_* ]] || \
      die "${BENCHMARK_SECRET_NAMESPACE}/${BENCHMARK_HF_SECRET_NAME} has no valid HF_TOKEN key"
    unset token
    log "using existing ${BENCHMARK_SECRET_NAMESPACE}/${BENCHMARK_HF_SECRET_NAME}"
    return
  fi

  [[ -n "${HF_TOKEN:-}" ]] || \
    die "set HF_TOKEN or create ${BENCHMARK_SECRET_NAMESPACE}/${BENCHMARK_HF_SECRET_NAME}"
  [[ "${HF_TOKEN}" == hf_* ]] || die "HF_TOKEN must start with hf_"
  printf 'HF_TOKEN=%s\n' "${HF_TOKEN}" |
    kubectl --context "${BENCHMARK_KUBE_CONTEXT}" \
      --namespace "${BENCHMARK_SECRET_NAMESPACE}" \
      create secret generic "${BENCHMARK_HF_SECRET_NAME}" \
      --from-env-file=/dev/stdin --dry-run=client -o yaml |
    kubectl --context "${BENCHMARK_KUBE_CONTEXT}" apply -f - >/dev/null
  log "created ${BENCHMARK_SECRET_NAMESPACE}/${BENCHMARK_HF_SECRET_NAME}"
}

finalize_campaign() {
  local campaign_status=$?
  local cleanup_status=0 destroy_status=0
  trap - EXIT INT TERM
  if [[ "${FINALIZER_ARMED}" == true ]]; then
    log "finalizing campaign ${BENCHMARK_CAMPAIGN_ID}"
    if [[ "${CAMPAIGN_CLEANUP_REQUIRED}" == true ]]; then
      run_make benchmark-gke-cleanup || cleanup_status=$?
    fi
    if [[ "${BENCHMARK_GKE_CLUSTER_LIFECYCLE}" == destroy ]]; then
      if (( cleanup_status == 0 )); then
        # A provisioning failure may occur before the cluster is created. The
        # finalizer still runs, so absence must be treated as the desired state.
        export BENCHMARK_GKE_DESTROY_IF_MISSING=true
        run_make benchmark-gke-destroy || destroy_status=$?
        unset BENCHMARK_GKE_DESTROY_IF_MISSING
      else
        log "ERROR: refusing cluster deletion because campaign cleanup failed" >&2
      fi
    fi
  fi
  if (( campaign_status != 0 )); then
    exit "${campaign_status}"
  fi
  if (( cleanup_status != 0 )); then
    exit "${cleanup_status}"
  fi
  exit "${destroy_status}"
}

main() {
  [[ $# -eq 0 ]] || die "run-gke-campaign.sh does not accept positional arguments"
  command -v "${MAKE_BIN}" >/dev/null 2>&1 || die "${MAKE_BIN} is required"
  command -v kubectl >/dev/null 2>&1 || die "kubectl is required"
  set_defaults

  log "campaign: ${BENCHMARK_CAMPAIGN_ID}"
  log "cluster lifecycle: ${BENCHMARK_GKE_CLUSTER_LIFECYCLE}"
  run_make benchmark-gke-plan

  # Ephemeral CI must also clean up a cluster created by a partially failing
  # provisioning operation. Retained clusters are armed after provisioning.
  if [[ "${BENCHMARK_GKE_CLUSTER_LIFECYCLE}" == destroy ]]; then
    FINALIZER_ARMED=true
    trap finalize_campaign EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
  fi
  run_make benchmark-gke-provision
  if [[ "${FINALIZER_ARMED}" != true ]]; then
    FINALIZER_ARMED=true
    trap finalize_campaign EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
  fi
  ensure_hf_secret

  local treatment
  for treatment in service agentgateway-standalone agentgateway-gateway; do
    # Scale-up itself is billable and must be reversed if the process is
    # interrupted before the benchmark creates its first namespace.
    CAMPAIGN_CLEANUP_REQUIRED=true
    log "acquiring GPU capacity for ${treatment}"
    run_make benchmark-gke-gpu-up
    log "running ${treatment}"
    export BENCHMARK_TREATMENT="${treatment}"
    run_make benchmark
  done
  unset BENCHMARK_TREATMENT

  local results_root campaign_dir
  results_root="${BENCHMARK_RESULTS_DIR:-${SCRIPT_DIR}/results/${BENCHMARK_SUITE}}"
  campaign_dir="${results_root}/${BENCHMARK_CAMPAIGN_ID}"
  export BENCHMARK_CAMPAIGN_DIR="${campaign_dir}"
  export BENCHMARK_COMPARISONS="service:agentgateway-standalone service:agentgateway-gateway"
  log "generating reports"
  run_make benchmark-report
  log "campaign evidence: ${campaign_dir}"
  log "generated reports: ${campaign_dir}/generated"
}

main "$@"
