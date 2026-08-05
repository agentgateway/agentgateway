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
kubectl exec -n "${NAMESPACE}" statefulset/redis-semantic-cache -- \
  redis-cli EVAL \
  "local k=redis.call('keys',ARGV[1]); if #k>0 then return redis.call('del',unpack(k)) end return 0" \
  0 'semantic-cache:*' >/dev/null

backend_count() {
  curl -fsS http://127.0.0.1:18081/stats | jq -er '.invocations'
}

cache_header() {
  awk 'BEGIN {IGNORECASE=1} /^x-vsr-cache-hit:/ {gsub("\r", "", $2); print tolower($2)}' "$1" | tail -n 1
}

decision_header() {
  awk 'BEGIN {IGNORECASE=1} /^x-vsr-selected-decision:/ {gsub("\r", "", $2); print $2}' "$1" | tail -n 1
}

send_request() {
  local name=$1
  local prompt=$2
  jq -n --arg prompt "${prompt}" '{
    model: "auto",
    messages: [{role: "user", content: $prompt}],
    max_tokens: 96
  }' >"${WORK_DIR}/${name}-request.json"
  curl -fsS -D "${WORK_DIR}/${name}-headers.txt" \
    -o "${WORK_DIR}/${name}-body.json" \
    http://127.0.0.1:18080/v1/chat/completions \
    -H 'Content-Type: application/json' \
    -H 'X-VSR-Debug: true' \
    -H "X-Request-ID: semantic-cache-${name}" \
    --data-binary "@${WORK_DIR}/${name}-request.json"
  jq -e '.choices[0].message.content | type == "string"' \
    "${WORK_DIR}/${name}-body.json" >/dev/null
}

assert_count() {
  local expected=$1
  local actual
  actual=$(backend_count)
  if [[ "${actual}" != "${expected}" ]]; then
    echo "expected backend count ${expected}, got ${actual}" >&2
    exit 1
  fi
}

assert_hit() {
  local name=$1
  local hit
  hit=$(cache_header "${WORK_DIR}/${name}-headers.txt")
  if [[ "${hit}" != true ]]; then
    echo "expected ${name} to be a cache hit" >&2
    sed -n '1,40p' "${WORK_DIR}/${name}-headers.txt" >&2
    exit 1
  fi
}

assert_uncached_decision() {
  local name=$1
  local decision
  decision=$(decision_header "${WORK_DIR}/${name}-headers.txt")
  if [[ "${decision}" != uncached_homehub_support ]]; then
    echo "expected ${name} to select uncached_homehub_support, got ${decision}" >&2
    exit 1
  fi
}

echo "Verifying initial miss"
send_request warm "How do I factory-reset my HomeHub X2?"
assert_count 1

echo "Verifying exact hit"
send_request exact "How do I factory-reset my HomeHub X2?"
assert_hit exact
assert_count 1

echo "Verifying paraphrase hit"
send_request paraphrase "What is the procedure for restoring a HomeHub X2 to factory settings?"
assert_hit paraphrase
assert_count 1

echo "Verifying semantically different miss"
send_request different "Can a HomeHub be used outdoors in freezing rain?"
assert_uncached_decision different
assert_count 2

echo "Inspecting the Redis Search index"
kubectl exec -n "${NAMESPACE}" statefulset/redis-semantic-cache -- \
  redis-cli FT.INFO semantic_cache_idx >/dev/null

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

echo "Semantic-cache verification passed"
