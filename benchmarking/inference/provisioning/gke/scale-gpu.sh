#!/usr/bin/env bash
# Resize the benchmark GPU pool and verify Kubernetes-visible capacity.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

usage() {
  cat <<'EOF'
Usage: scale-gpu.sh up|down|NODE_COUNT

"up" uses BENCHMARK_GKE_GPU_TARGET_NODES. A failed or timed-out scale-up is
automatically rolled back to zero nodes.
EOF
}

resize_pool() {
  local target="$1"
  gcloud container clusters resize "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --node-pool "${GKE_GPU_NODEPOOL}" --num-nodes "${target}" --quiet
}

node_capacity() {
  local output node ready accelerators nodes=0 ready_nodes=0 total_accelerators=0
  output="$(kubectl --context "${GKE_KUBE_CONTEXT}" get nodes \
    -l "cloud.google.com/gke-nodepool=${GKE_GPU_NODEPOOL}" \
    -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.status.conditions[?(@.type=="Ready")].status}{"\t"}{.status.allocatable.nvidia\.com/gpu}{"\n"}{end}')"
  while IFS=$'\t' read -r node ready accelerators; do
    [[ -n "${node}" ]] || continue
    nodes=$((nodes + 1))
    [[ "${ready}" == True ]] && ready_nodes=$((ready_nodes + 1))
    [[ "${accelerators:-}" =~ ^[0-9]+$ ]] && \
      total_accelerators=$((total_accelerators + accelerators))
  done <<<"${output}"
  echo "${nodes} ${ready_nodes} ${total_accelerators}"
}

wait_for_target() {
  local target="$1" timeout="$2"
  local deadline=$((SECONDS + timeout))
  local expected_accelerators=$((target * GKE_GPU_ACCELERATORS_PER_NODE))
  local capacity nodes ready accelerators
  while true; do
    if ! capacity="$(node_capacity)"; then
      log "ERROR: could not inspect Kubernetes GPU nodes" >&2
      return 2
    fi
    read -r nodes ready accelerators <<<"${capacity}"
    log "GPU pool target=${target}, nodes=${nodes}, ready=${ready}, allocatable-accelerators=${accelerators}/${expected_accelerators}"
    if [[ "${nodes}" == "${target}" && "${ready}" == "${target}" && \
          "${accelerators}" == "${expected_accelerators}" ]]; then
      return 0
    fi
    if (( SECONDS >= deadline )); then
      return 1
    fi
    sleep 10
  done
}

rollback_to_zero() {
  log "rolling GPU pool ${GKE_GPU_NODEPOOL} back to zero"
  resize_pool 0 || {
    log "ERROR: failed to request GPU rollback" >&2
    return 1
  }
  wait_for_target 0 600 || {
    log "ERROR: GPU nodes remain after rollback" >&2
    return 1
  }
}

main() {
  [[ $# -eq 1 ]] || { usage >&2; exit 2; }
  export CLOUDSDK_CORE_DISABLE_PROMPTS=1
  preflight
  cluster_exists || die "cluster ${GKE_CLUSTER} does not exist"
  assert_managed_cluster
  verify_gpu_pool
  get_credentials
  kubectl --context "${GKE_KUBE_CONTEXT}" cluster-info >/dev/null || \
    die "cannot reach ${GKE_KUBE_CONTEXT}"

  local target
  case "$1" in
    up) target="${GKE_GPU_TARGET_NODES}" ;;
    down) target=0 ;;
    *) target="$1" ;;
  esac
  is_nonnegative_integer "${target}" || { usage >&2; exit 2; }

  log "resizing ${GKE_GPU_NODEPOOL} to ${target} node(s)"
  if ! resize_pool "${target}"; then
    if (( target > 0 )); then
      rollback_to_zero || true
    fi
    die "GKE rejected the GPU node-pool resize"
  fi
  if ! wait_for_target "${target}" "${GKE_GPU_READY_TIMEOUT}"; then
    if (( target > 0 )); then
      rollback_to_zero || die "GPU scale-up timed out and rollback failed"
    fi
    die "GPU node pool did not reach ${target} Ready node(s) within ${GKE_GPU_READY_TIMEOUT}s"
  fi
  log "GPU node pool is ready at ${target} node(s)"
}

main "$@"
