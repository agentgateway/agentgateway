#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "${SCRIPT_DIR}"

if [[ ! -f .env ]]; then
  echo "copy env.example to .env and update it first" >&2
  exit 1
fi
chmod 600 .env

for command in docker envsubst openssl; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "required command not found: ${command}" >&2
    exit 1
  fi
done

if ! docker compose version >/dev/null 2>&1; then
  echo "Docker Compose v2 is required" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1091
source versions.env
if [[ -f runtime/generated.env ]]; then
  # shellcheck disable=SC1091
  source runtime/generated.env
fi
# shellcheck disable=SC1091
source .env
set +a

for variable in NETBIRD_MANAGEMENT_DOMAIN NETBIRD_PROXY_DOMAIN \
  NETBIRD_ADMIN_EMAIL NETBIRD_ADMIN_PASSWORD OPENAI_API_KEY ANTHROPIC_API_KEY; do
  value=${!variable:-}
  if [[ -z "${value}" ]]; then
    echo "required value is not set in .env: ${variable}" >&2
    exit 1
  fi
  if [[ "${value}" == replace-with-* ]]; then
    echo "replace the placeholder value in .env: ${variable}" >&2
    exit 1
  fi
done

mkdir -p runtime
chmod 700 runtime

secret_value() {
  local variable=$1
  local value=${!variable:-}
  if [[ -z "${value}" ]]; then
    case ${variable} in
      NETBIRD_VIRTUAL_KEY)
        value=$(openssl rand -hex 32)
        ;;
      *)
        value=$(openssl rand -base64 32)
        ;;
    esac
  fi
  printf '%s=%s\n' "${variable}" "${value}"
}

{
  printf 'AGENTGATEWAY_VERSION=%s\n' "${AGENTGATEWAY_VERSION}"
  printf 'NETBIRD_SERVER_IMAGE=%s\n' "${NETBIRD_SERVER_IMAGE}"
  printf 'NETBIRD_PROXY_IMAGE=%s\n' "${NETBIRD_PROXY_IMAGE}"
  printf 'NETBIRD_CLIENT_IMAGE=%s\n' "${NETBIRD_CLIENT_IMAGE}"
  printf 'NETBIRD_DASHBOARD_IMAGE=%s\n' "${NETBIRD_DASHBOARD_IMAGE}"
  secret_value NETBIRD_AUTH_SECRET
  secret_value NETBIRD_SESSION_KEY
  secret_value NETBIRD_STORE_KEY
  secret_value NETBIRD_VIRTUAL_KEY
} > runtime/generated.env
chmod 600 runtime/generated.env

set -a
# shellcheck disable=SC1091
source runtime/generated.env
set +a

envsubst < netbird-config.yaml.template > runtime/netbird-config.yaml
chmod 600 runtime/netbird-config.yaml

./generate-certificates.sh

docker compose --env-file .env --env-file runtime/generated.env config --quiet

echo
echo "Preparation complete. Trust runtime/certs/ca.crt on clients that access"
echo "the management dashboard or generated Agent Network endpoint."
