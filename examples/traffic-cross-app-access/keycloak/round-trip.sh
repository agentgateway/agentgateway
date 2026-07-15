#!/usr/bin/env bash
#
# Full ID-JAG round-trip against a single Keycloak (run after setup.sh + setup-leg2.sh):
#   0. password grant  -> alice's OIDC ID token (+ session)
#   1. token exchange  -> ID-JAG            (requested_token_type = ...:id-jag)
#   2. jwt-bearer grant -> Bearer access token  (assertion = the ID-JAG)
# Prints the decoded ID-JAG and the final access token.
set -euo pipefail

SERVER="${SERVER:-http://localhost:8480}"
REALM="${REALM:-idjag-demo}"
AGENT_SECRET="${KC_AGENT_SECRET:-agent-secret}"
RESOURCE_SECRET="${KC_RESOURCE_SECRET:-resource-secret}"
RESOURCE_ID="${RESOURCE_ID:-https://resource.idjag.demo}"
SCOPE="${SCOPE:-todos.read}"
TOKEN_URL="$SERVER/realms/$REALM/protocol/openid-connect/token"

decode() { python3 - "$1" <<'PY'
import sys,json,base64
h,p,_=sys.argv[1].split(".")
def d(s): return json.loads(base64.urlsafe_b64decode(s+"="*(-len(s)%4)))
print("  header :", json.dumps(d(h)))
print("  payload:", json.dumps(d(p), indent=2))
PY
}

echo "== 0) password grant -> ID token =="
ID_TOKEN=$(curl -s -X POST "$TOKEN_URL" -d grant_type=password -d client_id=agent-client \
  -d client_secret="$AGENT_SECRET" -d username=alice -d password=alice -d scope=openid \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["id_token"])')
echo "   ID token length: ${#ID_TOKEN}"

echo "== 1) token exchange -> ID-JAG =="
IDJAG=$(curl -s -X POST "$TOKEN_URL" -d grant_type=urn:ietf:params:oauth:grant-type:token-exchange \
  -d client_id=agent-client -d client_secret="$AGENT_SECRET" \
  --data-urlencode "subject_token=$ID_TOKEN" \
  -d subject_token_type=urn:ietf:params:oauth:token-type:id_token \
  -d requested_token_type=urn:ietf:params:oauth:token-type:id-jag \
  -d audience="$RESOURCE_ID" -d scope="$SCOPE" \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["access_token"])')
decode "$IDJAG"

echo "== 2) jwt-bearer -> final access token =="
RESP=$(curl -s -X POST "$TOKEN_URL" -d grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer \
  -d client_id=resource-client -d client_secret="$RESOURCE_SECRET" \
  --data-urlencode "assertion=$IDJAG" -d scope="$SCOPE")
echo "$RESP" | python3 -c 'import sys,json;d=json.load(sys.stdin);
print("  token_type:",d.get("token_type"),"| scope:",d.get("scope"),"| error:",d.get("error",""),d.get("error_description",""))'
AT=$(echo "$RESP" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("access_token",""))')
[ -n "$AT" ] && { echo "  final access token claims:"; decode "$AT" | sed -n '2,40p'; }
echo "ROUND-TRIP DONE"
