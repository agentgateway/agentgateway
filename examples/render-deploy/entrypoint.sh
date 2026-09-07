#!/bin/sh
# Prepare /config, then exec the official agentgateway binary as uid 65532.
# UI_PASSWORD is required. The file htpasswd is rewritten every start so
# dashboard env changes take effect. Inline bcrypt in config.yaml is a
# footgun: hashes contain $ and agentgateway env-expands $VARS. The htpasswd
# file is read as raw bytes, so bcrypt is safe there.
set -eu

# Pinned, not configurable: render.yaml mounts the disk at /config and the
# seeded config.yaml below refers to /config by absolute path.
CONFIG_DIR="/config"
CONFIG_FILE="${CONFIG_DIR}/config.yaml"
HTPASSWD_FILE="${CONFIG_DIR}/.htpasswd"
BIN="${AGENTGATEWAY_BIN:-/app/agentgateway}"

# The published image's uid. Render disks mount root-owned, so this image
# starts as root, chowns the disk, then drops to this uid.
RUN_UID=65532
RUN_GID=65532

UI_USER="${UI_USER:-admin}"
UI_PASSWORD="${UI_PASSWORD:-}"

if [ -z "${UI_PASSWORD}" ]; then
  echo "entrypoint: UI_PASSWORD is required to protect the UI; set it in the Render dashboard" >&2
  exit 1
fi

# $'\n' is a bashism; under dash (debian's /bin/sh) the old bracket
# expression matched a literal 'n' and rejected the default user "admin".
# Whitelist instead -- this also excludes ':' and newlines.
case "${UI_USER}" in
  ''|*[!A-Za-z0-9._@-]*)
    echo "entrypoint: UI_USER must be non-empty and use only letters, digits, '.', '_', '@' or '-'" >&2
    exit 1
    ;;
esac

mkdir -p "${CONFIG_DIR}"

# -B bcrypt, -C 10 cost, -i reads the password from stdin so it never lands
# in argv. {SHA} would be unsalted SHA-1; the htpasswd-verify fork accepts
# $2y$ bcrypt. It does not accept $6$, so openssl passwd is not a substitute.
umask 077
printf '%s\n' "${UI_PASSWORD}" | htpasswd -niB -C 10 "${UI_USER}" > "${HTPASSWD_FILE}"

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

# Root only on first boot to claim the disk; the gateway itself never runs
# as root. Skip both steps if the container was already started non-root.
if [ "$(id -u)" = "0" ]; then
  chown -R "${RUN_UID}:${RUN_GID}" "${CONFIG_DIR}"
  exec setpriv --reuid="${RUN_UID}" --regid="${RUN_GID}" --clear-groups \
    "${BIN}" -f "${CONFIG_FILE}"
fi

exec "${BIN}" -f "${CONFIG_FILE}"
