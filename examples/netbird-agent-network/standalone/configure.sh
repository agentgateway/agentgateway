#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "${SCRIPT_DIR}"

MODE=api
CHECK_ONLY=false
GROUP_NAME=${NETBIRD_GROUP_NAME:-agentgateway-clients}
PROVIDER_NAME=${NETBIRD_PROVIDER_NAME:-agentgateway}
POLICY_NAME=${NETBIRD_POLICY_NAME:-Agentgateway access}
UPSTREAM_URL=http://agent-network-agentgateway:3000
RESOURCE_NAME=netbird-agent-network-standalone

usage() {
  cat <<'EOF'
Usage: ./configure.sh [--mode api|dashboard] [--check]

Modes:
  api        Configure all resources through the API (default).
  dashboard  Configure prerequisites, then finish in the dashboard.

Options:
  --check    Validate the completed configuration without changing it.
  -h, --help Show this help.
EOF
}

while (($# > 0)); do
  case $1 in
    --mode)
      MODE=${2:?--mode requires api or dashboard}
      shift 2
      ;;
    --mode=*)
      MODE=${1#*=}
      shift
      ;;
    --check)
      CHECK_ONLY=true
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "unexpected argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ ${MODE} != api && ${MODE} != dashboard ]]; then
  echo "unsupported mode: ${MODE}; expected api or dashboard" >&2
  exit 1
fi

for command in curl docker jq; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "required command not found: ${command}" >&2
    exit 1
  fi
done

if [[ ! -f .env || ! -f runtime/generated.env ]]; then
  echo "run ./prepare.sh first" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1091
source .env
# shellcheck disable=SC1091
source runtime/generated.env
if [[ -f runtime/admin.env ]]; then
  # shellcheck disable=SC1091
  source runtime/admin.env
fi
set +a

COMPOSE=(docker compose --env-file .env --env-file runtime/generated.env)
MANAGEMENT_URL=https://${NETBIRD_MANAGEMENT_DOMAIN}
CA_CERT=runtime/certs/ca.crt

api() {
  local method=$1
  local path=$2
  local body=${3:-}
  local arguments=(
    --cacert "${CA_CERT}"
    -fsS
    -X "${method}"
    -H "Authorization: Token ${NETBIRD_PAT}"
    -H "Content-Type: application/json"
  )
  if [[ -n "${body}" ]]; then
    arguments+=(--data-binary "${body}")
  fi
  curl "${arguments[@]}" "${MANAGEMENT_URL}${path}"
}

wait_for_management() {
  echo "Waiting for NetBird management"
  for _ in $(seq 1 300); do
    if curl --cacert "${CA_CERT}" -fsS \
      "${MANAGEMENT_URL}/api/instance" >/dev/null 2>&1; then
      return
    fi
    sleep 2
  done
  echo "NetBird management did not become ready" >&2
  exit 1
}

check_configuration() {
  local settings providers groups policies provider group policy
  local endpoint provider_id group_id

  settings=$(api GET /api/agent-network/settings)
  endpoint=$(jq -er '.endpoint | select(length > 0)' <<<"${settings}") || {
    echo "Agent Network endpoint is not configured" >&2
    exit 1
  }

  providers=$(api GET /api/agent-network/providers)
  provider=$(jq -cer --arg name "${PROVIDER_NAME}" \
    'first(.[] | select(.name == $name))' <<<"${providers}") || {
    echo "provider not found: ${PROVIDER_NAME}" >&2
    exit 1
  }
  jq -e --arg upstream "${UPSTREAM_URL}" '
    .provider_id == "agentgateway" and
    .upstream_url == $upstream and
    .enabled == true and
    .metadata_disabled == false and
    .skip_tls_verification == false and
    (.models | length == 0)
  ' <<<"${provider}" >/dev/null || {
    echo "provider ${PROVIDER_NAME} does not match the expected configuration" >&2
    exit 1
  }
  provider_id=$(jq -er '.id' <<<"${provider}")

  groups=$(api GET /api/groups)
  group=$(jq -cer --arg name "${GROUP_NAME}" \
    'first(.[] | select(.name == $name))' <<<"${groups}") || {
    echo "group not found: ${GROUP_NAME}" >&2
    exit 1
  }
  group_id=$(jq -er '.id' <<<"${group}")

  policies=$(api GET /api/agent-network/policies)
  policy=$(jq -cer --arg name "${POLICY_NAME}" \
    'first(.[] | select(.name == $name))' <<<"${policies}") || {
    echo "policy not found: ${POLICY_NAME}" >&2
    exit 1
  }
  jq -e --arg group "${group_id}" --arg provider "${provider_id}" '
    .enabled == true and
    (.source_groups | index($group) != null) and
    (.destination_provider_ids | index($provider) != null)
  ' <<<"${policy}" >/dev/null || {
    echo "policy ${POLICY_NAME} does not authorize the expected resources" >&2
    exit 1
  }

  echo "Configuration complete."
  echo "Agent Network endpoint: https://${endpoint}"
}

if [[ ${CHECK_ONLY} == true ]]; then
  : "${NETBIRD_PAT:?NETBIRD_PAT is required for --check}"
  check_configuration
  exit 0
fi

"${COMPOSE[@]}" up -d certificate-init netbird-server netbird-dashboard \
  management-agentgateway agent-network-agentgateway
wait_for_management

if [[ -z "${NETBIRD_PAT:-}" ]]; then
  echo "Creating the initial NetBird owner and setup PAT"
  setup_body=$(jq -cn \
    --arg email "${NETBIRD_ADMIN_EMAIL}" \
    --arg password "${NETBIRD_ADMIN_PASSWORD}" '{
      email: $email,
      password: $password,
      name: "Agent Network Admin",
      create_pat: true,
      pat_expire_in: 30
    }')
  setup_response=$(curl --cacert "${CA_CERT}" -fsS \
    -H "Content-Type: application/json" --data-binary "${setup_body}" \
    "${MANAGEMENT_URL}/api/setup")
  NETBIRD_PAT=$(jq -er '.personal_access_token' <<<"${setup_response}")
  printf 'NETBIRD_PAT=%s\n' "${NETBIRD_PAT}" > runtime/admin.env
  chmod 600 runtime/admin.env
fi

if [[ ! -f runtime/proxy.env ]]; then
  echo "Creating a NetBird proxy access token"
  response=$(api POST /api/reverse-proxies/proxy-tokens \
    "{\"name\":\"${RESOURCE_NAME}\",\"expires_in\":0}")
  token=$(jq -er '.plain_token' <<<"${response}")
  printf 'NB_PROXY_TOKEN=%s\n' "${token}" > runtime/proxy.env
  chmod 600 runtime/proxy.env
  unset token response
fi

settings=$(api GET /api/agent-network/settings)
endpoint=$(jq -r '.endpoint // empty' <<<"${settings}")
if [[ -z "${endpoint}" ]]; then
  echo "Bootstrapping the Agent Network endpoint"
  body=$(jq -cn --arg proxy "${NETBIRD_PROXY_DOMAIN}" '{
    proxy_address: $proxy,
    enable_log_collection: true,
    enable_prompt_collection: false,
    redact_pii: false,
    access_log_retention_days: 7
  }')
  settings=$(api POST /api/agent-network/settings "${body}")
  endpoint=$(jq -er '.endpoint' <<<"${settings}")
fi

groups=$(api GET /api/groups)
group_id=$(jq -r --arg name "${GROUP_NAME}" \
  'first(.[] | select(.name == $name) | .id) // empty' <<<"${groups}")
if [[ -z "${group_id}" ]]; then
  echo "Creating the NetBird client group"
  body=$(jq -cn --arg name "${GROUP_NAME}" \
    '{name: $name, peers: [], resources: []}')
  group=$(api POST /api/groups "${body}")
  group_id=$(jq -er '.id' <<<"${group}")
fi

if [[ ! -f runtime/client.env ]]; then
  echo "Creating a one-use setup key for the example client"
  body=$(jq -cn --arg group "${group_id}" --arg name "${RESOURCE_NAME}" '{
    name: $name,
    type: "one-off",
    expires_in: 86400,
    auto_groups: [$group],
    usage_limit: 1,
    ephemeral: true,
    allow_extra_dns_labels: false
  }')
  response=$(api POST /api/setup-keys "${body}")
  key=$(jq -er '.key' <<<"${response}")
  printf 'NB_SETUP_KEY=%s\n' "${key}" > runtime/client.env
  chmod 600 runtime/client.env
  unset key response
fi

if [[ ${MODE} == api ]]; then
  providers=$(api GET /api/agent-network/providers)
  provider_id=$(jq -r --arg name "${PROVIDER_NAME}" \
    'first(.[] | select(.name == $name) | .id) // empty' <<<"${providers}")
  if [[ -z "${provider_id}" ]]; then
    echo "Creating the agentgateway provider"
    body=$(jq -cn --arg name "${PROVIDER_NAME}" \
      --arg upstream "${UPSTREAM_URL}" --arg key "${NETBIRD_VIRTUAL_KEY}" '{
        provider_id: "agentgateway",
        name: $name,
        upstream_url: $upstream,
        api_key: $key,
        models: [],
        enabled: true,
        skip_tls_verification: false,
        metadata_disabled: false
      }')
    provider=$(api POST /api/agent-network/providers "${body}")
    provider_id=$(jq -er '.id' <<<"${provider}")
  fi

  policies=$(api GET /api/agent-network/policies)
  policy_id=$(jq -r --arg name "${POLICY_NAME}" \
    'first(.[] | select(.name == $name) | .id) // empty' <<<"${policies}")
  if [[ -z "${policy_id}" ]]; then
    echo "Creating the Agent Network access policy"
    body=$(jq -cn --arg name "${POLICY_NAME}" --arg group "${group_id}" \
      --arg provider "${provider_id}" '{
        name: $name,
        description: "Allow the example client to use agentgateway",
        enabled: true,
        source_groups: [$group],
        destination_provider_ids: [$provider],
        guardrail_ids: []
      }')
    api POST /api/agent-network/policies "${body}" >/dev/null
  fi
fi

"${COMPOSE[@]}" --profile configured up -d netbird-proxy \
  netbird-example-client test-client

if [[ ${MODE} == dashboard ]]; then
  echo
  echo "Finish the provider and policy in https://${NETBIRD_MANAGEMENT_DOMAIN}:"
  echo "  Provider: ${PROVIDER_NAME} (${UPSTREAM_URL}), models empty, metadata enabled"
  echo "  Policy: ${POLICY_NAME}, ${GROUP_NAME} -> ${PROVIDER_NAME}"
  echo "Then run ./configure.sh --check."
  exit 0
fi

check_configuration
