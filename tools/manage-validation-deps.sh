#!/bin/bash

set -euo pipefail

KEYCLOAK_BASE_URL="http://localhost:7080"
VALIDATION_ENDPOINTS=(
  "http://localhost:9000/.well-known/jwks.json"
  # Wait for Keycloak JWKS endpoints to ensure realm import completed.
  "$KEYCLOAK_BASE_URL/realms/mcp/protocol/openid-connect/certs"
  "$KEYCLOAK_BASE_URL/realms/agentgateway/protocol/openid-connect/certs"
)

wait_for_http_ok() {
  local url="$1"
  local timeout_seconds="${2:-120}"
  local sleep_seconds="${3:-0.1}"

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

wait_for_dependencies() {
  for endpoint in "${VALIDATION_ENDPOINTS[@]}"; do
    if ! wait_for_http_ok "$endpoint" 240; then
      echo "Validation dependency did not become available in time: $endpoint" >&2
      return 1
    fi
  done
}

case "${1:-}" in
  start)
    export KEYCLOAK_MOCK_PORT="${KEYCLOAK_MOCK_PORT:-7080}"
    echo "Starting validation authentication server..."
    python3 examples/mcp-authentication/auth_server.py &

    wait_for_dependencies
    ;;
  stop)
    pkill -f "examples/mcp-authentication/auth_server.py" 2>/dev/null || true
    ;;
  *)
    echo "Usage: $0 {start|stop}"
    exit 1
    ;;
esac
