#!/bin/bash

set -euo pipefail

KEYCLOAK_BASE_URL="http://localhost:7080"
# Wait for JWKS to ensure realm import completed
KEYCLOAK_REALM_JWKS_ENDPOINT="$KEYCLOAK_BASE_URL/realms/mcp/protocol/openid-connect/certs"

wait_for_http_ok() {
  local url="$1"
  local timeout_seconds="${2:-120}"
  local sleep_seconds="${3:-2}"

  echo -n "Waiting for $url"
  local start_ts
  start_ts=$(date +%s)
  while true; do
    if curl -fsS "$url" >/dev/null 2>&1; then
      echo " - OK"
      return 0
    fi

    local now
    now=$(date +%s)
    if (( now - start_ts >= timeout_seconds )); then
      echo " - TIMEOUT after ${timeout_seconds}s"
      return 1
    fi

    echo -n "."
    sleep "$sleep_seconds"
  done
}

case "${1:-}" in
  start)
    echo "Starting MCP authentication server..."
    python3 examples/mcp-authentication/auth_server.py &

    echo "Starting Keycloak..."
    pushd examples/mcp-authentication/keycloak >/dev/null
    docker compose up -d
    popd >/dev/null

    # Realm import may complete after container is up; wait for JWKS specifically
    if ! wait_for_http_ok "$KEYCLOAK_REALM_JWKS_ENDPOINT" 240 3; then
      echo "Keycloak realm JWKS endpoint did not become available in time" >&2
      exit 1
    fi
    ;;
  stop)
    pkill -f "examples/mcp-authentication/auth_server.py" 2>/dev/null || true
    pushd examples/mcp-authentication/keycloak >/dev/null
    docker compose down
    popd >/dev/null
    ;;
  *)
    echo "Usage: $0 {start|stop}"
    exit 1
    ;;
esac
