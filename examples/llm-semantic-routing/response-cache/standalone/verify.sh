#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "${SCRIPT_DIR}"
RUN_SHARED_VSR=false
RUN_REDIS_RESTART=false
for argument in "$@"; do
  case "${argument}" in
    --shared-vsr) RUN_SHARED_VSR=true ;;
    --redis-restart) RUN_REDIS_RESTART=true ;;
    *) echo "usage: $0 [--shared-vsr] [--redis-restart]" >&2; exit 2 ;;
  esac
done
for command in docker curl jq; do
  command -v "${command}" >/dev/null || { echo "required command not found: ${command}" >&2; exit 1; }
done
compose() { docker compose --env-file versions.env "$@"; }
WORK_DIR=$(mktemp -d)
trap 'rm -rf -- "${WORK_DIR}"' EXIT
GATEWAY_URL="http://127.0.0.1:${PORT:-3000}"
# shellcheck source=examples/llm-semantic-routing/response-cache/shared/verify-homehub.sh
source "${SCRIPT_DIR}/../shared/verify-homehub.sh"
backend_request() {
  compose exec -T support-backend python -c '
import sys, urllib.request
request = urllib.request.Request("http://127.0.0.1:8080" + sys.argv[1], method=sys.argv[2])
print(urllib.request.urlopen(request, timeout=5).read().decode())
' "$1" "${2:-GET}"
}
backend_count() { backend_request /stats | jq -er '.invocations'; }
echo "Resetting the dedicated example data"
backend_request /admin/reset POST >/dev/null
redis_cli() { compose exec -T redis redis-cli "$@"; }
reset_cache
verify_requests
check_redis_indexes
if [[ "${RUN_SHARED_VSR}" == true ]]; then
  echo "Verifying that a fresh vSR instance reuses the Redis cache"
  # Keep the tested image for this check; do not fetch a different latest build.
  compose up -d --no-deps --force-recreate --pull never --wait --wait-timeout 650 semantic-router
  compose restart agentgateway
  send_request shared-vsr "How do I factory-reset my HomeHub X2?"
  assert_hit shared-vsr
  assert_count 2
fi
if [[ "${RUN_REDIS_RESTART}" == true ]]; then
  echo "Verifying persistence across a Redis restart"
  compose exec -T redis redis-cli SAVE >/dev/null
  compose restart redis
  compose up -d --no-deps --pull never --wait redis
  send_request redis-restart "How do I factory-reset my HomeHub X2?"
  assert_hit redis-restart
  assert_count 2
fi
echo "Response-cache verification passed"
