#!/usr/bin/env bash
set -euo pipefail

NAMESPACE=netbird-agent-network
PROVIDER_NAME=${NETBIRD_PROVIDER_NAME:-agentgateway}
POLICY_NAME=${NETBIRD_POLICY_NAME:-Agentgateway access}
RESOURCE_NAME=netbird-agent-network-example
CLEAN_MANAGEMENT=false

usage() {
  cat <<'EOF'
Usage: ./cleanup.sh [--management]

Options:
  --management  For a management database that survives namespace deletion,
                remove the example's account configuration first. Requires
                NETBIRD_MANAGEMENT_DOMAIN and NETBIRD_PAT.
  -h, --help    Show this help.
EOF
}

while (($# > 0)); do
  case $1 in
    --management)
      CLEAN_MANAGEMENT=true
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

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "required command not found: $1" >&2
    exit 1
  fi
}

require_variable() {
  if [[ -z "${!1:-}" ]]; then
    echo "required environment variable is not set: $1" >&2
    exit 1
  fi
}

require_command kubectl

api() {
  local method=$1
  local path=$2
  curl -fsS \
    -X "${method}" \
    -H "Authorization: Token ${NETBIRD_PAT}" \
    -H "Content-Type: application/json" \
    "https://${NETBIRD_MANAGEMENT_DOMAIN}${path}"
}

delete_named_resources() {
  local collection=$1
  local name=$2
  local label=$3
  local ids id

  ids=$(api GET "${collection}" | jq -r --arg name "${name}" \
    '.[] | select(.name == $name) | .id')
  while IFS= read -r id; do
    if [[ -z "${id}" ]]; then
      continue
    fi
    echo "Deleting ${label}: ${name}"
    api DELETE "${collection}/${id}" >/dev/null
  done <<<"${ids}"
}

cleanup_management() {
  require_command curl
  require_command jq
  require_variable NETBIRD_MANAGEMENT_DOMAIN
  require_variable NETBIRD_PAT

  echo "Checking NetBird management API access"
  api GET /api/agent-network/settings >/dev/null

  if kubectl get namespace "${NAMESPACE}" >/dev/null 2>&1; then
    echo "Stopping the example proxy and client"
    for deployment in netbird-proxy netbird-example-client; do
      if kubectl get deployment "${deployment}" -n "${NAMESPACE}" \
        >/dev/null 2>&1; then
        kubectl scale deployment "${deployment}" -n "${NAMESPACE}" \
          --replicas=0 >/dev/null
        kubectl rollout status deployment "${deployment}" -n "${NAMESPACE}" \
          --timeout=5m >/dev/null
      fi
    done
  fi

  delete_named_resources /api/agent-network/policies \
    "${POLICY_NAME}" "Agent Network policy"
  delete_named_resources /api/agent-network/providers \
    "${PROVIDER_NAME}" "Agent Network provider"
  delete_named_resources /api/setup-keys \
    "${RESOURCE_NAME}" "setup key"

  local tokens token_id settings
  tokens=$(api GET /api/reverse-proxies/proxy-tokens)
  while IFS= read -r token_id; do
    if [[ -z "${token_id}" ]]; then
      continue
    fi
    echo "Revoking proxy token: ${RESOURCE_NAME}"
    api DELETE "/api/reverse-proxies/proxy-tokens/${token_id}" >/dev/null
  done < <(jq -r --arg name "${RESOURCE_NAME}" \
    '.[] | select(.name == $name and .revoked == false) | .id' <<<"${tokens}")

  settings=$(api GET /api/agent-network/settings)
  if jq -e '.endpoint | length > 0' <<<"${settings}" >/dev/null; then
    echo "Deleting Agent Network settings"
    for _ in {1..10}; do
      if api DELETE /api/agent-network/settings >/dev/null 2>&1; then
        settings=
        break
      fi
      sleep 2
    done
    if [[ -n "${settings}" ]]; then
      echo "Agent Network settings are still in use; the final delete failed" >&2
      api DELETE /api/agent-network/settings >/dev/null
    fi
  fi

  echo "The example's managed NetBird account configuration is removed."
}

if [[ ${CLEAN_MANAGEMENT} == true ]]; then
  cleanup_management
fi

kubectl delete namespace "${NAMESPACE}" --ignore-not-found

echo "The Kubernetes resources are removed."
