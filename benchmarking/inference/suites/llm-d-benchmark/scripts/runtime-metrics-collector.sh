#!/usr/bin/env bash

# Runs llm-d-benchmark's direct vLLM/EPP collector and adds agentgateway plus
# Kubernetes resource snapshots to the same result tree. This stays separate
# from Prometheus discovery: EPP obtains routing signals directly from request
# lifecycle state and model-server endpoints, while these samples are evidence
# for post-run diagnosis.

set -euo pipefail

RESULTS_DIR="${LLMDBENCH_RUN_EXPERIMENT_RESULTS_DIR:-/runtime-metrics}"
NAMESPACE="${LLMDBENCH_VLLM_COMMON_NAMESPACE:?benchmark namespace is required}"
INTERVAL="${METRICS_COLLECTION_INTERVAL:-5}"
AGENTGATEWAY_PORT="${AGENTGATEWAY_METRICS_PORT:-15020}"
METRICS_PATH="${LLMDBENCH_VLLM_MONITORING_METRICS_PATH:-/metrics}"
STOP_FILE="${RUNTIME_METRICS_STOP_FILE:-/control/stop}"
COMPLETE_FILE="${RUNTIME_METRICS_COMPLETE_FILE:-/control/complete}"
COPIED_FILE="${RUNTIME_METRICS_COPIED_FILE:-/control/copied}"
COPY_WAIT_TIMEOUT="${RUNTIME_METRICS_COPY_WAIT_TIMEOUT:-300}"
UPSTREAM_COLLECTOR="${UPSTREAM_COLLECTOR:-/scripts/collect_metrics.sh}"
START_TIME="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
UPSTREAM_PID=""

log() { echo "[runtime-metrics] $*"; }

agentgateway_pods() {
  kubectl get pods --namespace "${NAMESPACE}" \
    --field-selector=status.phase=Running -o json | python3 -c '
import json
import sys

for pod in json.load(sys.stdin).get("items", []):
    metadata = pod.get("metadata", {})
    spec = pod.get("spec", {})
    labels = metadata.get("labels", {})
    containers = spec.get("containers", [])
    images = " ".join(str(c.get("image", "")) for c in containers).lower()
    names = " ".join(str(c.get("name", "")) for c in containers).lower()
    is_gateway = "agentgateway" in images or "agentgateway" in names
    is_gateway = is_gateway or "gateway.networking.k8s.io/gateway-name" in labels
    if is_gateway:
        name = metadata.get("name", "")
        address = pod.get("status", {}).get("podIP", "")
        if name and address:
            print(name, address)
'
}

scrape_agentgateway() {
  local timestamp iso_timestamp pod_name pod_ip output temporary
  timestamp="$(date +%s)"
  iso_timestamp="$(date -u +%Y-%m-%dT%H:%M:%S%z)"
  while read -r pod_name pod_ip; do
    [[ -n "${pod_name:-}" && -n "${pod_ip:-}" ]] || continue
    output="${RESULTS_DIR}/metrics/raw/${pod_name}_${timestamp}_agentgateway_metrics.log"
    temporary="${output}.tmp"
    {
      echo "# Timestamp: ${iso_timestamp}"
      echo "# Pod: ${pod_name}"
      echo "# PodIP: ${pod_ip}"
      echo "# Namespace: ${NAMESPACE}"
      echo "# Source: agentgateway_prometheus_metrics"
      echo
    } > "${output}"
    if curl --silent --show-error --connect-timeout 3 --max-time 10 \
        "http://${pod_ip}:${AGENTGATEWAY_PORT}${METRICS_PATH}" \
        > "${temporary}" 2>>"${RESULTS_DIR}/metrics/raw/collection_debug.log"; then
      cat "${temporary}" >> "${output}"
    else
      echo "# Warning: Failed to collect agentgateway metrics" >> "${output}"
    fi
    rm -f "${temporary}"
  done < <(agentgateway_pods)
}

collect_resource_snapshot() {
  local timestamp output
  timestamp="$(date +%s)"
  output="${RESULTS_DIR}/metrics/raw/container_resources_${timestamp}.txt"
  {
    echo "# Timestamp: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "# Namespace: ${NAMESPACE}"
    kubectl top pods --namespace "${NAMESPACE}" --containers --no-headers
  } > "${output}" 2>>"${RESULTS_DIR}/metrics/raw/collection_debug.log" || true
}

capture_logs() {
  local output
  mkdir -p "${RESULTS_DIR}/logs"
  kubectl logs --namespace "${NAMESPACE}" \
    --selector 'llm-d.ai/inferenceServing=true' --all-containers --prefix \
    --since-time "${START_TIME}" \
    > "${RESULTS_DIR}/logs/vllm_pods.log" 2>&1 || true
  kubectl logs --namespace "${NAMESPACE}" --selector inferencepool \
    --all-containers --prefix --since-time "${START_TIME}" \
    > "${RESULTS_DIR}/logs/epp_pods.log" 2>&1 || true

  # Gateway API data planes do not necessarily share EPP labels.
  output="${RESULTS_DIR}/logs/agentgateway_pods.log"
  : > "${output}"
  while read -r pod_name _; do
    kubectl logs --namespace "${NAMESPACE}" "${pod_name}" --all-containers \
      --prefix --since-time "${START_TIME}" >> "${output}" 2>&1 || true
  done < <(agentgateway_pods)
}

stop_upstream() {
  if [[ -n "${UPSTREAM_PID}" ]] && kill -0 "${UPSTREAM_PID}" 2>/dev/null; then
    kill "${UPSTREAM_PID}" 2>/dev/null || true
    wait "${UPSTREAM_PID}" 2>/dev/null || true
  fi
}

cleanup() {
  stop_upstream
}
trap cleanup EXIT INT TERM

mkdir -p "${RESULTS_DIR}/metrics/raw" "${RESULTS_DIR}/metrics/processed"
cat > "${RESULTS_DIR}/collection-window.yaml" <<EOF
start: "${START_TIME}"
namespace: "${NAMESPACE}"
interval_seconds: ${INTERVAL}
sources:
  - vllm
  - epp
  - agentgateway
  - kubernetes-resource-usage
EOF

log "starting upstream vLLM/EPP collector at ${INTERVAL}s intervals"
"${UPSTREAM_COLLECTOR}" start >> "${RESULTS_DIR}/metrics_collection.log" 2>&1 &
UPSTREAM_PID=$!

while [[ ! -e "${STOP_FILE}" ]]; do
  scrape_agentgateway
  collect_resource_snapshot
  sleep "${INTERVAL}" &
  wait $! || true
done

stop_upstream
UPSTREAM_PID=""
capture_logs
log "processing combined vLLM, EPP, and agentgateway samples"
"${UPSTREAM_COLLECTOR}" process >> "${RESULTS_DIR}/metrics_collection.log" 2>&1
if [[ -f /scripts/process_epp_logs.py ]]; then
  python3 /scripts/process_epp_logs.py "${RESULTS_DIR}" \
    >> "${RESULTS_DIR}/metrics_collection.log" 2>&1 || \
    log "warning: EPP log processing failed; raw logs remain available"
fi
cat >> "${RESULTS_DIR}/collection-window.yaml" <<EOF
stop: "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
EOF
log "collection complete"
touch "${COMPLETE_FILE}"

# kubectl cp uses exec and therefore cannot read files after a pod enters the
# Succeeded phase. Keep the collector alive until the wrapper acknowledges that
# it copied the completed result tree, with a bounded wait for interrupted runs.
copy_wait_deadline=$((SECONDS + COPY_WAIT_TIMEOUT))
while [[ ! -e "${COPIED_FILE}" && ${SECONDS} -lt ${copy_wait_deadline} ]]; do
  sleep 1
done
if [[ ! -e "${COPIED_FILE}" ]]; then
  log "warning: timed out waiting for runtime metrics copy acknowledgement"
fi
