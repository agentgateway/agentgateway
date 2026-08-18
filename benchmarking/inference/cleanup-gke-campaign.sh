#!/usr/bin/env bash
# Clean resources owned by one GKE inference benchmark campaign and always
# scale its GPU node pool to zero. Intended for an unconditional CI finalizer.

set -uo pipefail

BENCHMARK_KUBE_CONTEXT="${BENCHMARK_KUBE_CONTEXT:-}"
BENCHMARK_CAMPAIGN_ID="${BENCHMARK_CAMPAIGN_ID:-}"
GKE_PROJECT="${BENCHMARK_GKE_PROJECT:-}"
GKE_LOCATION="${BENCHMARK_GKE_LOCATION:-}"
GKE_CLUSTER="${BENCHMARK_GKE_CLUSTER:-}"
GKE_GPU_NODEPOOL="${BENCHMARK_GKE_GPU_NODEPOOL:-gpu-h100}"
CLEANUP_TIMEOUT="${BENCHMARK_GKE_CLEANUP_TIMEOUT:-1200}"
MANAGED_BY="agentgateway-benchmark"
LOCK_NAMESPACE="agentgateway-benchmark-system"
LOCK_NAME="campaign-lock"
FAILED=0

log() { echo "[benchmark-cleanup] $*"; }
fail() { log "ERROR: $*" >&2; FAILED=1; }
k() { kubectl --context "${BENCHMARK_KUBE_CONTEXT}" "$@"; }

require_value() {
  local name="$1" value="$2"
  [[ -n "${value}" ]] || { fail "${name} is required"; return 1; }
}

append_unique() {
  local array_name="$1" value="$2" existing
  case "${array_name}" in
    PVS)
      for existing in "${PVS[@]}"; do
        [[ "${existing}" != "${value}" ]] || return
      done
      PVS+=("${value}")
      ;;
    FILESTORE_INSTANCES)
      for existing in "${FILESTORE_INSTANCES[@]}"; do
        [[ "${existing}" != "${value}" ]] || return
      done
      FILESTORE_INSTANCES+=("${value}")
      ;;
    *)
      fail "internal error: unsupported cleanup array ${array_name}"
      ;;
  esac
}

resolve_identity() {
  if [[ "${BENCHMARK_KUBE_CONTEXT}" =~ ^gke_([^_]+)_([^_]+)_(.+)$ ]]; then
    [[ -n "${GKE_PROJECT}" ]] || GKE_PROJECT="${BASH_REMATCH[1]}"
    [[ -n "${GKE_LOCATION}" ]] || GKE_LOCATION="${BASH_REMATCH[2]}"
    [[ -n "${GKE_CLUSTER}" ]] || GKE_CLUSTER="${BASH_REMATCH[3]}"
  fi
  require_value BENCHMARK_KUBE_CONTEXT "${BENCHMARK_KUBE_CONTEXT}" || true
  require_value BENCHMARK_CAMPAIGN_ID "${BENCHMARK_CAMPAIGN_ID}" || true
  require_value BENCHMARK_GKE_PROJECT "${GKE_PROJECT}" || true
  require_value BENCHMARK_GKE_LOCATION "${GKE_LOCATION}" || true
  require_value BENCHMARK_GKE_CLUSTER "${GKE_CLUSTER}" || true
  require_value BENCHMARK_GKE_GPU_NODEPOOL "${GKE_GPU_NODEPOOL}" || true
  [[ "${CLEANUP_TIMEOUT}" =~ ^[1-9][0-9]*$ ]] || \
    fail "BENCHMARK_GKE_CLEANUP_TIMEOUT must be a positive integer"
}

discover_namespaces() {
  local namespace campaign output
  output="$(k get namespaces \
    -l "app.kubernetes.io/managed-by=${MANAGED_BY}" \
    -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.metadata.annotations.benchmark\.agentgateway\.dev/campaign-id}{"\n"}{end}')" || {
      fail "could not list owned benchmark namespaces"
      return
    }
  while IFS=$'\t' read -r namespace campaign; do
    [[ -n "${namespace}" ]] || continue
    if [[ "${campaign}" == "${BENCHMARK_CAMPAIGN_ID}" ]]; then
      CAMPAIGN_NAMESPACES+=("${namespace}")
    fi
  done <<<"${output}"
}

record_pv() {
  local namespace="$1" pv="$2"
  [[ -n "${pv}" ]] || return
  local claim_namespace driver handle
  claim_namespace="$(k get pv "${pv}" -o jsonpath='{.spec.claimRef.namespace}' 2>/dev/null || true)"
  if [[ "${claim_namespace}" != "${namespace}" ]]; then
    fail "PV ${pv} does not belong to owned namespace ${namespace}"
    return
  fi
  driver="$(k get pv "${pv}" -o jsonpath='{.spec.csi.driver}' 2>/dev/null || true)"
  case "${driver}" in
    filestore.csi.storage.gke.io)
      handle="$(k get pv "${pv}" -o jsonpath='{.spec.csi.volumeHandle}')"
      if [[ "${handle}" =~ ^modeInstance/([^/]+)/([^/]+)/[^/]+$ ]]; then
        append_unique FILESTORE_INSTANCES "${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
      else
        fail "unrecognized Filestore volume handle for ${pv}: ${handle}"
        return
      fi
      ;;
    pd.csi.storage.gke.io) ;;
    *)
      fail "PV ${pv} has unexpected CSI driver ${driver:-none}"
      return
      ;;
  esac
  append_unique PVS "${pv}"
}

collect_storage() {
  local namespace pvc pv pvc_output pv_output
  for namespace in "${CAMPAIGN_NAMESPACES[@]}"; do
    pvc_output="$(k get pvc --namespace "${namespace}" \
      -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.spec.volumeName}{"\n"}{end}')" || {
        fail "could not list PVCs in ${namespace}"
        continue
      }
    while IFS=$'\t' read -r pvc pv; do
      [[ -n "${pvc}" ]] || continue
      record_pv "${namespace}" "${pv}"
    done <<<"${pvc_output}"
    pv_output="$(k get pv \
      -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.spec.claimRef.namespace}{"\n"}{end}')" || {
        fail "could not list PVs for ${namespace}"
        continue
      }
    while IFS= read -r pv; do
      [[ -n "${pv}" ]] || continue
      record_pv "${namespace}" "${pv}"
    done < <(awk -v namespace="${namespace}" '$2 == namespace {print $1}' <<<"${pv_output}")
  done
}

discover_filestore_instances() {
  (( ${#CAMPAIGN_NAMESPACES[@]} > 0 )) || return
  local inventory output item
  inventory="$(gcloud filestore instances list --project "${GKE_PROJECT}" \
    --location "${GKE_LOCATION}" --format=json)" || {
      fail "could not list Filestore instances in ${GKE_LOCATION}"
      return
    }
  output="$(python3 -c '
import json
import sys

namespaces = set(sys.argv[1:])
for instance in json.load(sys.stdin):
    labels = instance.get("labels", {})
    if labels.get("kubernetes_io_created-for_pvc_namespace") not in namespaces:
        continue
    parts = instance["name"].split("/")
    print(f"{parts[-3]}/{parts[-1]}")
' "${CAMPAIGN_NAMESPACES[@]}" <<<"${inventory}")" || {
    fail "could not inspect Filestore ownership labels"
    return
  }
  while IFS= read -r item; do
    [[ -n "${item}" ]] || continue
    append_unique FILESTORE_INSTANCES "${item}"
  done <<<"${output}"
}

delete_storage_and_namespaces() {
  local namespace pv deadline=$((SECONDS + CLEANUP_TIMEOUT))
  for namespace in "${CAMPAIGN_NAMESPACES[@]}"; do
    log "deleting PVCs and namespace ${namespace}"
    k delete pvc --all --namespace "${namespace}" --ignore-not-found \
      --wait=false >/dev/null 2>&1 || fail "could not delete PVCs in ${namespace}"
    k delete namespace "${namespace}" --ignore-not-found \
      --wait=false >/dev/null 2>&1 || fail "could not delete namespace ${namespace}"
  done

  for pv in "${PVS[@]}"; do
    k delete pv "${pv}" --ignore-not-found --wait=false >/dev/null 2>&1 || \
      fail "could not request deletion of PV ${pv}"
  done
  for pv in "${PVS[@]}"; do
    local remaining=$((deadline - SECONDS))
    (( remaining > 0 )) || remaining=1
    k wait --for=delete "pv/${pv}" --timeout="${remaining}s" >/dev/null 2>&1 || \
      fail "timed out or failed waiting for PV ${pv} deletion"
  done
  for namespace in "${CAMPAIGN_NAMESPACES[@]}"; do
    local remaining=$((deadline - SECONDS))
    (( remaining > 0 )) || remaining=1
    k wait --for=delete "namespace/${namespace}" \
      --timeout="${remaining}s" >/dev/null 2>&1 || \
      fail "timed out or failed waiting for namespace ${namespace} deletion"
  done
}

verify_filestore_deleted() {
  local item location instance describe_output describe_status
  local deadline=$((SECONDS + CLEANUP_TIMEOUT))
  for item in "${FILESTORE_INSTANCES[@]}"; do
    location="${item%%/*}"
    instance="${item#*/}"
    while true; do
      describe_status=0
      describe_output="$(gcloud filestore instances describe "${instance}" \
        --project "${GKE_PROJECT}" --location "${location}" 2>&1)" || \
        describe_status=$?
      if (( describe_status != 0 )); then
        if grep -Eqi 'NOT_FOUND|not found' <<<"${describe_output}"; then
          break
        fi
        fail "could not verify Filestore instance ${location}/${instance}: ${describe_output}"
        break
      fi
      if (( SECONDS >= deadline )); then
        fail "Filestore instance ${location}/${instance} still exists"
        break
      fi
      sleep 10
    done
  done
}

scale_gpu_pool_to_zero() {
  log "validating GPU node pool ${GKE_PROJECT}/${GKE_LOCATION}/${GKE_CLUSTER}/${GKE_GPU_NODEPOOL}"
  local pool
  pool="$(gcloud container node-pools describe "${GKE_GPU_NODEPOOL}" \
    --cluster "${GKE_CLUSTER}" --project "${GKE_PROJECT}" \
    --location "${GKE_LOCATION}" --format='value(name)' 2>/dev/null || true)"
  if [[ "${pool}" != "${GKE_GPU_NODEPOOL}" ]]; then
    fail "could not validate GPU node pool ${GKE_GPU_NODEPOOL}"
    return
  fi
  log "scaling GPU node pool ${GKE_GPU_NODEPOOL} to zero"
  gcloud container clusters resize "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --node-pool "${GKE_GPU_NODEPOOL}" --num-nodes 0 --quiet || {
      fail "GPU node-pool resize failed"
      return
    }

  local node_count nodes_output deadline=$((SECONDS + CLEANUP_TIMEOUT))
  while true; do
    # Use the name output format because some kubectl versions write
    # "No resources found" when --no-headers returns an empty result. Since
    # stderr is captured to preserve diagnostic errors, that notice would be
    # miscounted as a remaining GPU node and make successful cleanup time out.
    nodes_output="$(k get nodes \
      -l "cloud.google.com/gke-nodepool=${GKE_GPU_NODEPOOL}" \
      -o name 2>&1)" || {
        fail "could not verify Kubernetes GPU nodes: ${nodes_output}"
        break
      }
    node_count="$(awk 'NF {count++} END {print count+0}' <<<"${nodes_output}")"
    [[ "${node_count}" != "0" ]] || break
    if (( SECONDS >= deadline )); then
      fail "${node_count} GPU node(s) remain"
      break
    fi
    sleep 10
  done

  local url zone group target urls_output
  urls_output="$(gcloud container node-pools describe "${GKE_GPU_NODEPOOL}" \
    --cluster "${GKE_CLUSTER}" --project "${GKE_PROJECT}" \
    --location "${GKE_LOCATION}" --format='value(instanceGroupUrls[])')" || {
      fail "could not list GPU managed instance groups"
      return
    }
  while IFS= read -r url; do
    [[ -n "${url}" ]] || continue
    zone="$(sed -E 's#^.*/zones/([^/]+)/.*#\1#' <<<"${url}")"
    group="${url##*/}"
    target="$(gcloud compute instance-groups managed describe "${group}" \
      --project "${GKE_PROJECT}" --zone "${zone}" \
      --format='value(targetSize)' 2>/dev/null || true)"
    [[ "${target}" == "0" ]] || fail "GPU managed instance group ${zone}/${group} target size is ${target:-unknown}"
  done <<<"${urls_output}"
}

release_campaign_lock() {
  (( FAILED == 0 )) || return
  local holder output status=0
  output="$(k get configmap "${LOCK_NAME}" --namespace "${LOCK_NAMESPACE}" \
    -o jsonpath='{.data.campaign-id}' 2>&1)" || status=$?
  if (( status != 0 )); then
    if grep -Eqi 'NotFound|not found' <<<"${output}"; then
      return
    fi
    fail "could not inspect campaign lock: ${output}"
    return
  fi
  holder="${output}"
  if [[ "${holder}" != "${BENCHMARK_CAMPAIGN_ID}" ]]; then
    fail "refusing to release lock held by campaign ${holder}"
    return
  fi
  k delete configmap "${LOCK_NAME}" --namespace "${LOCK_NAMESPACE}" \
    --wait=true >/dev/null || fail "could not release campaign lock"
}

main() {
  resolve_identity
  command -v kubectl >/dev/null || fail "kubectl is required"
  command -v gcloud >/dev/null || fail "gcloud is required"
  command -v python3 >/dev/null || fail "python3 is required"
  if (( FAILED != 0 )); then
    return "${FAILED}"
  fi
  k cluster-info >/dev/null 2>&1 || fail "cannot reach ${BENCHMARK_KUBE_CONTEXT}"

  declare -ga CAMPAIGN_NAMESPACES=() PVS=() FILESTORE_INSTANCES=()
  (( FAILED != 0 )) || discover_namespaces
  (( FAILED != 0 )) || collect_storage
  (( FAILED != 0 )) || discover_filestore_instances
  (( FAILED != 0 )) || delete_storage_and_namespaces
  (( FAILED != 0 )) || verify_filestore_deleted

  # GPU scale-down is deliberately attempted even if Kubernetes or storage
  # cleanup failed, so the most expensive resource does not remain running.
  scale_gpu_pool_to_zero
  release_campaign_lock
  if (( FAILED != 0 )); then
    log "cleanup failed"
    return 1
  fi
  log "cleanup verified: no campaign storage remains and GPU pool is at zero"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
