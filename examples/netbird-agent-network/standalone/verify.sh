#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "${SCRIPT_DIR}"

RUN_LIVE_PROVIDER_TESTS=${RUN_LIVE_PROVIDER_TESTS:-false}
OPENAI_MODEL=${OPENAI_MODEL:-gpt-4o-mini}
ANTHROPIC_MODEL=${ANTHROPIC_MODEL:-claude-haiku-4-5}
EMBEDDING_MODEL=${EMBEDDING_MODEL:-text-embedding-3-small}

for command in curl docker jq; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "required command not found: ${command}" >&2
    exit 1
  fi
done

if [[ ! -f .env || ! -f runtime/generated.env || ! -f runtime/admin.env ]]; then
  echo "run ./prepare.sh and ./configure.sh first" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1091
source .env
# shellcheck disable=SC1091
source runtime/generated.env
# shellcheck disable=SC1091
source runtime/admin.env
set +a

COMPOSE=(docker compose --env-file .env --env-file runtime/generated.env)
CA_CERT=runtime/certs/ca.crt

api() {
  curl --cacert "${CA_CERT}" -fsS \
    -H "Authorization: Token ${NETBIRD_PAT}" \
    "https://${NETBIRD_MANAGEMENT_DOMAIN}$1"
}

assert_status() {
  local expected=$1
  shift
  local actual
  actual=$(curl -sS -o /dev/null -w '%{http_code}' "$@")
  if [[ ${actual} != "${expected}" ]]; then
    echo "expected HTTP ${expected}, got ${actual}: curl $*" >&2
    exit 1
  fi
}

client_curl() {
  "${COMPOSE[@]}" exec -T test-client curl -fsS "$@"
}

echo "Checking the public management listener"
assert_status 200 --cacert "${CA_CERT}" \
  "https://${NETBIRD_MANAGEMENT_DOMAIN}/api/instance"

echo "Checking the relay WebSocket upgrade"
status=$(curl --cacert "${CA_CERT}" -s --http1.1 --max-time 2 \
  -o /dev/null -w '%{http_code}' \
  -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Key: MDEyMzQ1Njc4OWFiY2RlZg==' \
  -H 'Sec-WebSocket-Version: 13' \
  "https://${NETBIRD_MANAGEMENT_DOMAIN}/relay" || true)
if [[ ${status} != 101 ]]; then
  echo "expected WebSocket upgrade HTTP 101, got ${status}" >&2
  exit 1
fi

echo "Checking that the private agentgateway requires the virtual key"
"${COMPOSE[@]}" --profile diagnostics up -d private-test-client >/dev/null
private_status=$("${COMPOSE[@]}" exec -T private-test-client \
  curl -sS -o /dev/null -w '%{http_code}' \
  http://agent-network-agentgateway:3000/v1/models)
if [[ ${private_status} != 401 ]]; then
  echo "expected private agentgateway HTTP 401, got ${private_status}" >&2
  exit 1
fi
private_status=$("${COMPOSE[@]}" exec -T private-test-client \
  curl -sS -o /dev/null -w '%{http_code}' \
  -H 'Authorization: Bearer invalid-key' \
  http://agent-network-agentgateway:3000/v1/models)
if [[ ${private_status} != 401 ]]; then
  echo "expected invalid virtual key HTTP 401, got ${private_status}" >&2
  exit 1
fi

echo "Checking that the NetBird proxy has no published TCP listener"
if "${COMPOSE[@]}" port netbird-proxy 443 2>/dev/null | grep -q .; then
  echo "the NetBird proxy must not publish TCP port 443" >&2
  exit 1
fi

endpoint=$(api /api/agent-network/settings | jq -er '.endpoint')

echo "Checking model discovery through NetBird"
client_curl "https://${endpoint}/v1/models" \
  | jq -e '.data | type == "array"' >/dev/null

if [[ ${RUN_LIVE_PROVIDER_TESTS} != true ]]; then
  echo "Skipping billable calls. Set RUN_LIVE_PROVIDER_TESTS=true to run them."
  echo "Non-billable NetBird/agentgateway verification passed."
  exit 0
fi

echo "Checking an OpenAI chat completion"
body=$(jq -cn --arg model "${OPENAI_MODEL}" '{
  model: $model,
  messages: [{role: "user", content: "Reply with the word connected."}],
  max_tokens: 16
}')
client_curl "https://${endpoint}/v1/chat/completions" \
  -H 'Content-Type: application/json' --data-binary "${body}" \
  | jq -e '.choices[0].message.content | type == "string"' >/dev/null

echo "Checking an OpenAI Responses request"
body=$(jq -cn --arg model "${OPENAI_MODEL}" '{
  model: $model,
  input: "Reply with the word connected.",
  max_output_tokens: 16
}')
client_curl "https://${endpoint}/v1/responses" \
  -H 'Content-Type: application/json' --data-binary "${body}" \
  | jq -e '.output | type == "array"' >/dev/null

echo "Checking an OpenAI embedding"
body=$(jq -cn --arg model "${EMBEDDING_MODEL}" \
  '{model: $model, input: "NetBird and agentgateway"}')
client_curl "https://${endpoint}/v1/embeddings" \
  -H 'Content-Type: application/json' --data-binary "${body}" \
  | jq -e '.data[0].embedding | type == "array"' >/dev/null

echo "Checking an Anthropic message"
body=$(jq -cn --arg model "${ANTHROPIC_MODEL}" '{
  model: $model,
  max_tokens: 16,
  messages: [{role: "user", content: "Reply with the word connected."}]
}')
client_curl "https://${endpoint}/v1/messages" \
  -H 'Content-Type: application/json' --data-binary "${body}" \
  | jq -e '.content | type == "array"' >/dev/null

echo "Checking OpenAI streaming"
body=$(jq -cn --arg model "${OPENAI_MODEL}" '{
  model: $model,
  messages: [{role: "user", content: "Reply with the word connected."}],
  max_tokens: 16,
  stream: true
}')
client_curl -N "https://${endpoint}/v1/chat/completions" \
  -H 'Content-Type: application/json' --data-binary "${body}" \
  | grep '^data:' >/dev/null

echo "Checking Anthropic streaming"
body=$(jq -cn --arg model "${ANTHROPIC_MODEL}" '{
  model: $model,
  max_tokens: 16,
  messages: [{role: "user", content: "Reply with the word connected."}],
  stream: true
}')
client_curl -N "https://${endpoint}/v1/messages" \
  -H 'Content-Type: application/json' --data-binary "${body}" \
  | grep '^data:' >/dev/null

echo "Live NetBird/agentgateway verification passed."
