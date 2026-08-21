#!/usr/bin/env bash
# Delete an idle benchmark cluster created by provision.sh.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

verify_cluster_is_idle() {
  local namespaces pvs load_balancers
  namespaces="$(kubectl --context "${GKE_KUBE_CONTEXT}" get namespaces \
    -l "app.kubernetes.io/managed-by=${MANAGED_BY}" -o name)"
  [[ -z "${namespaces}" ]] || \
    die "benchmark namespaces remain; run benchmark-gke-cleanup before destroying the cluster"
  pvs="$(kubectl --context "${GKE_KUBE_CONTEXT}" get pv -o name)"
  [[ -z "${pvs}" ]] || \
    die "persistent volumes remain; clean and verify their cloud storage before destroying the cluster"
  load_balancers="$(kubectl --context "${GKE_KUBE_CONTEXT}" get services --all-namespaces \
    -o jsonpath='{range .items[?(@.spec.type=="LoadBalancer")]}{.metadata.namespace}{"/"}{.metadata.name}{"\n"}{end}')"
  [[ -z "${load_balancers}" ]] || \
    die "LoadBalancer services remain: ${load_balancers//$'\n'/, }"
}

remove_kubeconfig_entries() {
  kubectl config delete-context "${GKE_KUBE_CONTEXT}" >/dev/null 2>&1 || true
  kubectl config delete-cluster "${GKE_KUBE_CONTEXT}" >/dev/null 2>&1 || true
  kubectl config delete-user "${GKE_KUBE_CONTEXT}" >/dev/null 2>&1 || true
}

main() {
  export CLOUDSDK_CORE_DISABLE_PROMPTS=1
  preflight
  [[ "${BENCHMARK_GKE_ALLOW_DESTROY:-false}" == true ]] || \
    die "set BENCHMARK_GKE_ALLOW_DESTROY=true to delete the cluster"
  validate_bool BENCHMARK_GKE_DESTROY_IF_MISSING \
    "${BENCHMARK_GKE_DESTROY_IF_MISSING:-false}"
  if ! cluster_exists; then
    if [[ "${BENCHMARK_GKE_DESTROY_IF_MISSING:-false}" == true ]]; then
      remove_kubeconfig_entries
      log "cluster ${GKE_CLUSTER} is already absent"
      return
    fi
    die "cluster ${GKE_CLUSTER} does not exist"
  fi
  assert_managed_cluster
  if node_pool_exists "${GKE_GPU_NODEPOOL}"; then
    verify_gpu_pool
  else
    log "GPU node pool ${GKE_GPU_NODEPOOL} is already absent"
  fi
  get_credentials
  verify_cluster_is_idle

  log "deleting ${GKE_PROJECT}/${GKE_LOCATION}/${GKE_CLUSTER}"
  gcloud container clusters delete "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" --quiet
  cluster_exists && die "cluster still exists after the delete operation"
  remove_kubeconfig_entries
  log "cluster deleted; project IAM and service accounts were retained"
}

main "$@"
