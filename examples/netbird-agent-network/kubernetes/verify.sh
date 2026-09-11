#!/usr/bin/env bash
set -euo pipefail

NAMESPACE=netbird-agent-network
AGENTGATEWAY_NAMESPACE=netbird-agent-network
RUN_LIVE_PROVIDER_TESTS=${RUN_LIVE_PROVIDER_TESTS:-false}
OPENAI_MODEL=${OPENAI_MODEL:-gpt-4o-mini}
ANTHROPIC_MODEL=${ANTHROPIC_MODEL:-claude-haiku-4-5}

for command in curl jq kubectl; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "required command not found: ${command}" >&2
    exit 1
  fi
done

for variable in NETBIRD_AGENT_ENDPOINT NETBIRD_MANAGEMENT_DOMAIN NETBIRD_VIRTUAL_KEY; do
  if [[ -z "${!variable:-}" ]]; then
    echo "required environment variable is not set: ${variable}" >&2
    exit 1
  fi
done

WORK_DIR=$(mktemp -d)
PORT_FORWARD_PID=

cleanup() {
  if [[ -n "${PORT_FORWARD_PID}" ]]; then
    kill "${PORT_FORWARD_PID}" 2>/dev/null || true
  fi
  case "${WORK_DIR}" in
    /tmp/*|/private/var/folders/*|/var/folders/*)
      rm -rf -- "${WORK_DIR}"
      ;;
  esac
}
trap cleanup EXIT INT TERM

assert_status() {
  local expected=$1
  shift
  local actual
  actual=$(curl -sS -o /dev/null -w '%{http_code}' "$@")
  if [[ "${actual}" != "${expected}" ]]; then
    echo "expected HTTP ${expected}, got ${actual}: curl $*" >&2
    exit 1
  fi
}

assert_websocket_upgrade() {
  local url=$1
  local actual
  actual=$(curl -s --http1.1 --max-time 2 -o /dev/null -w '%{http_code}' \
    -H 'Connection: Upgrade' \
    -H 'Upgrade: websocket' \
    -H 'Sec-WebSocket-Key: MDEyMzQ1Njc4OWFiY2RlZg==' \
    -H 'Sec-WebSocket-Version: 13' \
    "${url}" || true)
  if [[ "${actual}" != "101" ]]; then
    echo "expected WebSocket upgrade HTTP 101, got ${actual}: ${url}" >&2
    exit 1
  fi
}

echo "Checking workload readiness"
kubectl wait --for=condition=Programmed gateway/netbird-agentgateway \
  -n "${AGENTGATEWAY_NAMESPACE}" --timeout=300s
kubectl wait --for=condition=Programmed gateway/netbird-management \
  -n "${AGENTGATEWAY_NAMESPACE}" --timeout=300s
kubectl wait --for=condition=Ready certificate/netbird-management \
  -n "${AGENTGATEWAY_NAMESPACE}" --timeout=300s
kubectl rollout status deployment/netbird-agentgateway \
  -n "${AGENTGATEWAY_NAMESPACE}" --timeout=300s
kubectl rollout status deployment/netbird-management \
  -n "${AGENTGATEWAY_NAMESPACE}" --timeout=300s
kubectl rollout status deployment/netbird-server \
  -n "${NAMESPACE}" --timeout=300s
kubectl rollout status deployment/netbird-proxy \
  -n "${NAMESPACE}" --timeout=300s
kubectl rollout status deployment/netbird-example-client \
  -n "${NAMESPACE}" --timeout=300s

echo "Checking the public management gateway"
assert_status 200 "https://${NETBIRD_MANAGEMENT_DOMAIN}/api/instance"
assert_status 308 "http://${NETBIRD_MANAGEMENT_DOMAIN}/api/instance"

echo "Checking the relay WebSocket upgrade"
assert_websocket_upgrade "https://${NETBIRD_MANAGEMENT_DOMAIN}/relay"

kubectl port-forward -n "${AGENTGATEWAY_NAMESPACE}" \
  service/netbird-agentgateway 18080:80 \
  >"${WORK_DIR}/port-forward.log" 2>&1 &
PORT_FORWARD_PID=$!

for _ in $(seq 1 30); do
  if curl -sS -o /dev/null http://127.0.0.1:18080/v1/models 2>/dev/null; then
    break
  fi
  sleep 1
done

echo "Checking strict virtual-key authentication"
assert_status 401 http://127.0.0.1:18080/v1/models
assert_status 401 http://127.0.0.1:18080/v1/models \
  -H 'Authorization: Bearer invalid-key'

echo "Checking that an unauthenticated public request cannot bypass NetBird"
assert_status 403 "https://${NETBIRD_AGENT_ENDPOINT}/v1/models"

if [[ "${RUN_LIVE_PROVIDER_TESTS}" != true ]]; then
  echo "Skipping billable provider calls. Set RUN_LIVE_PROVIDER_TESTS=true to run them."
  echo "Non-billable NetBird/agentgateway verification passed"
  exit 0
fi

echo "Checking OpenAI model listing through NetBird"
kubectl exec -n "${NAMESPACE}" deployment/netbird-example-client \
  -c test -- curl -fsS "https://${NETBIRD_AGENT_ENDPOINT}/v1/models" \
  | jq -e '.data | type == "array"' >/dev/null

echo "Checking an OpenAI chat completion through NetBird"
openai_body=$(jq -cn --arg model "${OPENAI_MODEL}" '{
  model: $model,
  messages: [{role: "user", content: "Reply with the word connected."}],
  max_tokens: 16
}')
kubectl exec -n "${NAMESPACE}" deployment/netbird-example-client \
  -c test -- curl -fsS "https://${NETBIRD_AGENT_ENDPOINT}/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  --data-binary "${openai_body}" \
  | jq -e '.choices[0].message.content | type == "string"' >/dev/null

echo "Checking an Anthropic message through NetBird"
anthropic_body=$(jq -cn --arg model "${ANTHROPIC_MODEL}" '{
  model: $model,
  max_tokens: 16,
  messages: [{role: "user", content: "Reply with the word connected."}]
}')
kubectl exec -n "${NAMESPACE}" deployment/netbird-example-client \
  -c test -- curl -fsS "https://${NETBIRD_AGENT_ENDPOINT}/v1/messages" \
  -H 'Content-Type: application/json' \
  --data-binary "${anthropic_body}" \
  | jq -e '.content | type == "array"' >/dev/null

echo "Checking OpenAI streaming through NetBird"
stream_body=$(jq -cn --arg model "${OPENAI_MODEL}" '{
  model: $model,
  messages: [{role: "user", content: "Reply with the word connected."}],
  max_tokens: 16,
  stream: true
}')
kubectl exec -n "${NAMESPACE}" deployment/netbird-example-client \
  -c test -- curl -fsS -N \
  "https://${NETBIRD_AGENT_ENDPOINT}/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  --data-binary "${stream_body}" | grep '^data:' >/dev/null

echo "Checking Anthropic streaming through NetBird"
anthropic_stream_body=$(jq -cn --arg model "${ANTHROPIC_MODEL}" '{
  model: $model,
  max_tokens: 16,
  messages: [{role: "user", content: "Reply with the word connected."}],
  stream: true
}')
kubectl exec -n "${NAMESPACE}" deployment/netbird-example-client \
  -c test -- curl -fsS -N \
  "https://${NETBIRD_AGENT_ENDPOINT}/v1/messages" \
  -H 'Content-Type: application/json' \
  --data-binary "${anthropic_stream_body}" | grep '^data:' >/dev/null

echo "Live NetBird/agentgateway verification passed"
