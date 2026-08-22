#!/usr/bin/env bash
#
# Mint an ID-JAG (leg 1) from the Keycloak set up by setup.sh:
#   1. password grant with the requesting client -> user's OIDC ID token (+ session)
#   2. RFC 8693 token exchange, requested_token_type=id-jag -> the ID-JAG
# Then decode and print the ID-JAG header + payload.
set -euo pipefail

SERVER="${SERVER:-http://localhost:8480}"
REALM="${REALM:-idjag-demo}"
AGENT_SECRET="${KC_AGENT_SECRET:-agent-secret}"
RESOURCE_ID="${RESOURCE_ID:-https://resource.idjag.demo}"
SCOPE="${SCOPE:-todos.read}"
TOKEN_URL="$SERVER/realms/$REALM/protocol/openid-connect/token"

echo "== 1) password grant -> ID token (creates a user session) =="
ID_TOKEN=$(curl -s -X POST "$TOKEN_URL" \
  -d grant_type=password -d client_id=agent-client -d client_secret="$AGENT_SECRET" \
  -d username=alice -d password=alice -d scope=openid \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["id_token"])')
echo "   ID token length: ${#ID_TOKEN}"

echo "== 2) token exchange -> ID-JAG =="
IDJAG=$(curl -s -X POST "$TOKEN_URL" \
  -d grant_type=urn:ietf:params:oauth:grant-type:token-exchange \
  -d client_id=agent-client -d client_secret="$AGENT_SECRET" \
  --data-urlencode "subject_token=$ID_TOKEN" \
  -d subject_token_type=urn:ietf:params:oauth:token-type:id_token \
  -d requested_token_type=urn:ietf:params:oauth:token-type:id-jag \
  -d audience="$RESOURCE_ID" \
  -d scope="$SCOPE" \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["access_token"])')

echo "== ID-JAG decoded =="
python3 - "$IDJAG" <<'PY'
import sys,json,base64
def dec(seg): return json.loads(base64.urlsafe_b64decode(seg+"="*(-len(seg)%4)))
h,p,_=sys.argv[1].split(".")
print("header :", json.dumps(dec(h)))
print("payload:", json.dumps(dec(p),indent=2))
PY
