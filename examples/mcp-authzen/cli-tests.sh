#!/usr/bin/env bash
# Call one MCP tool through agentgateway as a given user. Each tool has a
# canned set of arguments below, so you don't need to pass any JSON for the
# common case - only if you want to override it.
set -euo pipefail

usage() {
  cat <<EOF
Usage: $0 <user> <tool> [json-args]

  ./cli-tests.sh alice list_accounts     # allowed - any role
  ./cli-tests.sh alice create_payment    # allowed - creates pay-1
  ./cli-tests.sh alice approve_payment   # denied  - payments_user can't approve
  ./cli-tests.sh bob   approve_payment   # allowed - payments_manager can

  ./cli-tests.sh bob approve_payment '{"payment_id":"pay-2"}'   # override the args

Users: alice (payments_user)  bob (payments_manager)  carol (payments_admin)
Tools: list_accounts  get_account  create_payment  approve_payment  cancel_payment
EOF
}

if [[ $# -lt 2 ]]; then
  usage
  exit 1
fi

USER="$1"
TOOL="$2"
ARGS="${3:-}"
if [[ -z "$ARGS" ]]; then
  case "$TOOL" in
    list_accounts)   ARGS='{}' ;;
    get_account)     ARGS='{"account_id":"acct-checking"}' ;;
    create_payment)  ARGS='{"account_id":"acct-checking","amount":42,"recipient":"bob"}' ;;
    approve_payment) ARGS='{"payment_id":"pay-1"}' ;;
    cancel_payment)  ARGS='{"payment_id":"pay-1"}' ;;
    *)               ARGS='{}' ;;
  esac
fi

TOKEN=$(curl -s http://localhost:7080/realms/mcp/protocol/openid-connect/token \
  -u mcp-client:mcp-client-secret \
  -d grant_type=password -d "username=$USER" -d "password=${USER}123" -d scope=openid \
  | jq -r .access_token)

# MCP over streamable HTTP is session-based: initialize first and reuse the
# Mcp-Session-Id it returns for the actual tool call.
SESSION=$(curl -si http://localhost:3000/mcp \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"cli-tests","version":"1.0"}}}' \
  | grep -i '^mcp-session-id:' | tr -d '\r' | awk '{print $2}')

curl -s -o /dev/null http://localhost:3000/mcp \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' -H "mcp-session-id: $SESSION" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'

echo "user=$USER tool=$TOOL args=$ARGS"
curl -s -w '\nHTTP %{http_code}\n' http://localhost:3000/mcp \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' -H "mcp-session-id: $SESSION" \
  -d "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"$TOOL\",\"arguments\":$ARGS}}"
