#!/bin/sh
# Prepare /config, then exec the official agentgateway binary.
# UI_PASSWORD is required. The file htpasswd is rewritten every start so
# dashboard env changes take effect. Inline bcrypt in config.yaml is a
# footgun: hashes contain $ and agentgateway env-expands $VARS.
set -eu

CONFIG_DIR="${AGW_CONFIG_DIR:-/config}"
CONFIG_FILE="${CONFIG_DIR}/config.yaml"
HTPASSWD_FILE="${CONFIG_DIR}/.htpasswd"
BIN="${AGENTGATEWAY_BIN:-/app/agentgateway}"

UI_USER="${UI_USER:-admin}"
UI_PASSWORD="${UI_PASSWORD:-}"

if [ -z "${UI_PASSWORD}" ]; then
  echo "entrypoint: UI_PASSWORD is required to protect the UI; set it in the Render dashboard" >&2
  exit 1
fi

case "${UI_USER}" in
  *:*|*[$'\n\r']*)
    echo "entrypoint: UI_USER must not contain ':' or newlines" >&2
    exit 1
    ;;
esac

mkdir -p "${CONFIG_DIR}"

hash="$(printf '%s' "${UI_PASSWORD}" | openssl dgst -binary -sha1 | openssl enc -base64 -A)"
printf '%s:{SHA}%s\n' "${UI_USER}" "${hash}" > "${HTPASSWD_FILE}"
chmod 600 "${HTPASSWD_FILE}"

if [ ! -f "${CONFIG_FILE}" ]; then
  cat > "${CONFIG_FILE}" <<'EOF'
# yaml-language-server: $schema=https://agentgateway.dev/schema/config
config:
  database:
    url: sqlite:///config/data.db
gateways:
  default:
    port: 4000
ui:
  gateways: [default]
  policies:
    basicAuth:
      mode: strict
      htpasswd:
        file: /config/.htpasswd
      realm: agentgateway
EOF
fi

exec "${BIN}" -f "${CONFIG_FILE}"
