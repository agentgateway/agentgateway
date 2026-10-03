#!/usr/bin/env bash
set -euo pipefail

NAMESPACE=agentgateway-system
BACKEND_NAMESPACE=homehub
RUN_SHARED_VSR=false
RUN_REDIS_RESTART=false

for argument in "$@"; do
  case "${argument}" in
    --shared-vsr)
      RUN_SHARED_VSR=true
      ;;
    --redis-restart)
      RUN_REDIS_RESTART=true
      ;;
    *)
      echo "usage: $0 [--shared-vsr] [--redis-restart]" >&2
      exit 2
      ;;
  esac
done

for command in kubectl curl jq; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "required command not found: ${command}" >&2
    exit 1
  fi
done

WORK_DIR=$(mktemp -d)
GATEWAY_PID=
BACKEND_PID=
SCALED_VSR=false

cleanup() {
  if [[ -n "${GATEWAY_PID}" ]]; then
    kill "${GATEWAY_PID}" 2>/dev/null || true
  fi
  if [[ -n "${BACKEND_PID}" ]]; then
    kill "${BACKEND_PID}" 2>/dev/null || true
  fi
  if [[ "${SCALED_VSR}" == true ]]; then
    kubectl scale deployment/semantic-router -n "${NAMESPACE}" \
      --replicas=1 >/dev/null 2>&1 || true
  fi
  case "${WORK_DIR}" in
    /tmp/*|/private/var/folders/*|/var/folders/*)
      rm -rf -- "${WORK_DIR}"
      ;;
  esac
}
trap cleanup EXIT INT TERM

echo "Checking workload readiness"
kubectl wait --for=condition=Available deployment/semantic-router \
  -n "${NAMESPACE}" --timeout=600s
kubectl wait --for=condition=Available deployment/support-backend \
  -n "${BACKEND_NAMESPACE}" --timeout=120s
kubectl rollout status statefulset/redis-semantic-cache \
  -n "${NAMESPACE}" --timeout=120s
kubectl wait --for=condition=Programmed gateway/agentgateway-proxy \
  -n "${NAMESPACE}" --timeout=300s

kubectl port-forward -n "${NAMESPACE}" service/agentgateway-proxy \
  18080:80 >"${WORK_DIR}/gateway-port-forward.log" 2>&1 &
GATEWAY_PID=$!
kubectl port-forward -n "${BACKEND_NAMESPACE}" service/support-backend \
  18081:8080 >"${WORK_DIR}/backend-port-forward.log" 2>&1 &
BACKEND_PID=$!

for _ in $(seq 1 30); do
  if curl -sS http://127.0.0.1:18081/healthz >/dev/null 2>&1 && \
      curl -sS -o /dev/null http://127.0.0.1:18080/ 2>/dev/null; then
    break
  fi
  sleep 1
done
curl -fsS http://127.0.0.1:18081/healthz >/dev/null

echo "Resetting the dedicated example data"
curl -fsS -X POST http://127.0.0.1:18081/admin/reset >/dev/null
backend_count() {
  curl -fsS http://127.0.0.1:18081/stats | jq -er '.invocations'
}

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
GATEWAY_URL=http://127.0.0.1:18080
# shellcheck source=examples/llm-semantic-routing/response-cache/shared/verify-homehub.sh
source "${SCRIPT_DIR}/../shared/verify-homehub.sh"
redis_cli() { kubectl exec -n "${NAMESPACE}" statefulset/redis-semantic-cache -- redis-cli "$@"; }
reset_cache
verify_requests

echo "Inspecting the Redis Search index"
check_redis_indexes

if [[ "${RUN_SHARED_VSR}" == true ]]; then
  echo "Verifying cache sharing across vSR replicas"
  original_vsr=$(kubectl get pods -n "${NAMESPACE}" \
    -l app.kubernetes.io/instance=semantic-router \
    -o jsonpath='{.items[0].metadata.name}')
  if [[ -z "${original_vsr}" ]]; then
    echo "could not identify the original vSR pod" >&2
    exit 1
  fi
  kubectl scale deployment/semantic-router -n "${NAMESPACE}" --replicas=2
  SCALED_VSR=true
  kubectl rollout status deployment/semantic-router \
    -n "${NAMESPACE}" --timeout=600s
  kubectl delete pod "${original_vsr}" -n "${NAMESPACE}" --wait=true
  kubectl rollout status deployment/semantic-router \
    -n "${NAMESPACE}" --timeout=600s
  send_request shared-vsr \
    "How do I factory-reset my HomeHub X2?"
  assert_hit shared-vsr
  assert_count 2
fi

if [[ "${RUN_REDIS_RESTART}" == true ]]; then
  echo "Verifying persistence across a Redis pod restart"
  kubectl exec -n "${NAMESPACE}" statefulset/redis-semantic-cache -- \
    redis-cli SAVE >/dev/null
  redis_pod=$(kubectl get pod -n "${NAMESPACE}" \
    -l app.kubernetes.io/name=redis-semantic-cache \
    -o jsonpath='{.items[0].metadata.name}')
  kubectl delete pod "${redis_pod}" -n "${NAMESPACE}" --wait=true
  kubectl rollout status statefulset/redis-semantic-cache \
    -n "${NAMESPACE}" --timeout=180s
  send_request redis-restart \
    "How do I factory-reset my HomeHub X2?"
  assert_hit redis-restart
  assert_count 2
fi

echo "Response-cache verification passed"
