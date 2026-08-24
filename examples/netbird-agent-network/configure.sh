#!/usr/bin/env bash
set -euo pipefail

NAMESPACE=netbird-agent-network
GROUP_NAME=${NETBIRD_GROUP_NAME:-agentgateway-clients}
PROVIDER_NAME=${NETBIRD_PROVIDER_NAME:-agentgateway}
POLICY_NAME=${NETBIRD_POLICY_NAME:-Agentgateway access}
UPSTREAM_URL=${AGENTGATEWAY_UPSTREAM_URL:-http://netbird-agentgateway.netbird-agent-network.svc.cluster.local}
MODE=api
CHECK_ONLY=false

usage() {
  cat <<'EOF'
Usage: ./configure.sh [--mode api|dashboard] [--check]

Modes:
  api        Configure all resources through the API (default).
  dashboard  Configure shared prerequisites, then finish in the dashboard.

Options:
  --check    Read-only validation of the completed configuration. Requires
             NETBIRD_PAT.
  -h, --help Show this help.
EOF
}

while (($# > 0)); do
  case $1 in
    --mode)
      if (($# < 2)); then
        echo "--mode requires api or dashboard" >&2
        exit 1
      fi
      MODE=$2
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

for command in curl jq kubectl; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "required command not found: ${command}" >&2
    exit 1
  fi
done

if [[ -z "${NETBIRD_MANAGEMENT_DOMAIN:-}" ]]; then
  echo "required environment variable is not set: NETBIRD_MANAGEMENT_DOMAIN" >&2
  exit 1
fi

MANAGEMENT_URL="https://${NETBIRD_MANAGEMENT_DOMAIN}"

api() {
  local method=$1
  local path=$2
  local body=${3:-}
  local arguments=(
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

check_configuration() {
  local settings providers groups policies provider group policy
  local endpoint provider_id group_id

  echo "Checking the completed NetBird configuration"
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
  if ! jq -e --arg upstream "${UPSTREAM_URL}" '
    .provider_id == "agentgateway" and
    .upstream_url == $upstream and
    .enabled == true and
    .metadata_disabled == false and
    .skip_tls_verification == false and
    (.models | length == 0)
  ' <<<"${provider}" >/dev/null; then
    echo "provider ${PROVIDER_NAME} does not match the expected agentgateway configuration" >&2
    exit 1
  fi
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
  if ! jq -e \
    --arg group "${group_id}" \
    --arg provider "${provider_id}" '
      .enabled == true and
      (.source_groups | index($group) != null) and
      (.destination_provider_ids | index($provider) != null)
    ' <<<"${policy}" >/dev/null; then
    echo "policy ${POLICY_NAME} does not authorize the expected group and provider" >&2
    exit 1
  fi

  for secret in netbird-proxy-auth netbird-client-setup; do
    if ! kubectl get secret "${secret}" -n "${NAMESPACE}" \
      >/dev/null 2>&1; then
      echo "Kubernetes Secret not found: ${secret}" >&2
      exit 1
    fi
  done

  echo
  echo "Configuration complete."
  echo "Agent Network endpoint: https://${endpoint}"
  echo "Export this value before running verify.sh:"
  echo "export NETBIRD_AGENT_ENDPOINT=${endpoint}"
}

if [[ ${CHECK_ONLY} == true ]]; then
  if [[ -z "${NETBIRD_PAT:-}" ]]; then
    echo "required environment variable is not set: NETBIRD_PAT" >&2
    exit 1
  fi
  check_configuration
  exit 0
fi

for variable in NETBIRD_PROXY_DOMAIN NETBIRD_VIRTUAL_KEY; do
  if [[ -z "${!variable:-}" ]]; then
    echo "required environment variable is not set: ${variable}" >&2
    exit 1
  fi
done

if [[ -z "${NETBIRD_PAT:-}" ]]; then
  for variable in NETBIRD_ADMIN_EMAIL NETBIRD_ADMIN_PASSWORD; do
    if [[ -z "${!variable:-}" ]]; then
      echo "set NETBIRD_PAT, or set ${variable} for initial setup" >&2
      exit 1
    fi
  done

  echo "Creating the initial NetBird owner and temporary setup PAT"
  setup_body=$(jq -cn \
    --arg email "${NETBIRD_ADMIN_EMAIL}" \
    --arg password "${NETBIRD_ADMIN_PASSWORD}" \
    '{
      email: $email,
      password: $password,
      name: "Agent Network Admin",
      create_pat: true,
      pat_expire_in: 30
    }')
  setup_response=$(curl -fsS \
    -H "Content-Type: application/json" \
    --data-binary "${setup_body}" \
    "${MANAGEMENT_URL}/api/setup")
  NETBIRD_PAT=$(jq -er '.personal_access_token' <<<"${setup_response}")
fi

if ! kubectl get secret netbird-proxy-auth -n "${NAMESPACE}" \
  >/dev/null 2>&1; then
  echo "Creating a NetBird proxy access token"
  proxy_token_response=$(api POST /api/reverse-proxies/proxy-tokens \
    '{"name":"netbird-agent-network-example","expires_in":0}')
  proxy_token=$(jq -er '.plain_token' <<<"${proxy_token_response}")
  kubectl create secret generic netbird-proxy-auth \
    -n "${NAMESPACE}" \
    --from-literal=token="${proxy_token}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  unset proxy_token proxy_token_response
fi

if [[ ${MODE} == api ]]; then
  settings=$(api GET /api/agent-network/settings)
  endpoint=$(jq -r '.endpoint // empty' <<<"${settings}")
  if [[ -z "${endpoint}" ]]; then
    echo "Bootstrapping the Agent Network endpoint"
    settings_body=$(jq -cn --arg proxy "${NETBIRD_PROXY_DOMAIN}" '{
      proxy_address: $proxy,
      enable_log_collection: true,
      enable_prompt_collection: false,
      redact_pii: false,
      access_log_retention_days: 7
    }')
    settings=$(api POST /api/agent-network/settings "${settings_body}")
    endpoint=$(jq -er '.endpoint' <<<"${settings}")
  fi

  providers=$(api GET /api/agent-network/providers)
  provider_id=$(jq -r --arg name "${PROVIDER_NAME}" \
    'first(.[] | select(.name == $name) | .id) // empty' <<<"${providers}")
  if [[ -z "${provider_id}" ]]; then
    echo "Creating the NetBird agentgateway provider"
    provider_body=$(jq -cn \
      --arg name "${PROVIDER_NAME}" \
      --arg upstream "${UPSTREAM_URL}" \
      --arg key "${NETBIRD_VIRTUAL_KEY}" \
      '{
        provider_id: "agentgateway",
        name: $name,
        upstream_url: $upstream,
        api_key: $key,
        models: [],
        enabled: true,
        skip_tls_verification: false,
        metadata_disabled: false
      }')
    provider=$(api POST /api/agent-network/providers "${provider_body}")
    provider_id=$(jq -er '.id' <<<"${provider}")
  fi
fi

groups=$(api GET /api/groups)
group_id=$(jq -r --arg name "${GROUP_NAME}" \
  'first(.[] | select(.name == $name) | .id) // empty' <<<"${groups}")
if [[ -z "${group_id}" ]]; then
  echo "Creating the NetBird client group"
  group_body=$(jq -cn --arg name "${GROUP_NAME}" \
    '{name: $name, peers: [], resources: []}')
  group=$(api POST /api/groups "${group_body}")
  group_id=$(jq -er '.id' <<<"${group}")
fi

if [[ ${MODE} == api ]]; then
  policies=$(api GET /api/agent-network/policies)
  policy_id=$(jq -r --arg name "${POLICY_NAME}" \
    'first(.[] | select(.name == $name) | .id) // empty' <<<"${policies}")
  if [[ -z "${policy_id}" ]]; then
    echo "Creating the Agent Network access policy"
    policy_body=$(jq -cn \
      --arg name "${POLICY_NAME}" \
      --arg group "${group_id}" \
      --arg provider "${provider_id}" \
      '{
        name: $name,
        description: "Allow the example client to use agentgateway",
        enabled: true,
        source_groups: [$group],
        destination_provider_ids: [$provider],
        guardrail_ids: []
      }')
    api POST /api/agent-network/policies "${policy_body}" >/dev/null
  fi
fi

if ! kubectl get secret netbird-client-setup -n "${NAMESPACE}" \
  >/dev/null 2>&1; then
  echo "Creating a one-use setup key for the example client"
  setup_key_body=$(jq -cn --arg group "${group_id}" '{
    name: "netbird-agent-network-example",
    type: "one-off",
    expires_in: 86400,
    auto_groups: [$group],
    usage_limit: 1,
    ephemeral: true,
    allow_extra_dns_labels: false
  }')
  setup_key_response=$(api POST /api/setup-keys "${setup_key_body}")
  setup_key=$(jq -er '.key' <<<"${setup_key_response}")
  kubectl create secret generic netbird-client-setup \
    -n "${NAMESPACE}" \
    --from-literal=setup-key="${setup_key}" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  unset setup_key setup_key_response
fi

kubectl rollout restart deployment/netbird-proxy deployment/netbird-example-client \
  -n "${NAMESPACE}" >/dev/null

if [[ ${MODE} == dashboard ]]; then
  kubectl rollout status deployment/netbird-proxy \
    -n "${NAMESPACE}" --timeout=5m >/dev/null
  echo
  echo "Shared prerequisites are configured. Complete these steps in the NetBird dashboard:"
  echo "  1. Open https://${NETBIRD_MANAGEMENT_DOMAIN}/agent-network/providers."
  echo "  2. Add the agentgateway provider named '${PROVIDER_NAME}'."
  echo "     Upstream URL: ${UPSTREAM_URL}"
  echo "     API key: the current NETBIRD_VIRTUAL_KEY value"
  echo "     Models: empty (allow all); identity metadata: enabled"
  echo "  3. Create the '${POLICY_NAME}' policy."
  echo "     Source group: ${GROUP_NAME}; destination provider: ${PROVIDER_NAME}"
  echo "  4. Export a NetBird PAT and run ./configure.sh --check."
  echo
  echo "Create the following DNS records if they do not exist:"
  echo "  ${NETBIRD_MANAGEMENT_DOMAIN} -> netbird-management LoadBalancer address"
  echo "  ${NETBIRD_PROXY_DOMAIN} -> netbird-proxy LoadBalancer address"
  echo "  *.${NETBIRD_PROXY_DOMAIN} -> ${NETBIRD_PROXY_DOMAIN}"
  exit 0
fi

check_configuration

echo
echo "Create the following DNS records if they do not exist:"
echo "  ${NETBIRD_MANAGEMENT_DOMAIN} -> netbird-management LoadBalancer address"
echo "  ${NETBIRD_PROXY_DOMAIN} -> netbird-proxy LoadBalancer address"
echo "  *.${NETBIRD_PROXY_DOMAIN} -> ${NETBIRD_PROXY_DOMAIN}"
