#!/usr/bin/env bash
#
# Obtain an IdenX (xaa.dev) ID token for the registered requesting app via the
# Authorization Code + PKCE flow. IdenX login is interactive, so this script:
#   1. generates a PKCE verifier/challenge and prints an authorize URL,
#   2. you open it, sign in (any email — no real account), and get redirected to
#      http://localhost:8500/callback?code=...  (the page won't load; that's fine),
#   3. you paste the `code` value back here,
#   4. it exchanges the code for tokens and prints the id_token.
#
# Requires: openssl, curl, python3. Reads the IdP client secret from ./secrets/idp-client-secret.
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"

CLIENT_ID="${CLIENT_ID:-client_411c671d8d1f8a69}"
REDIRECT_URI="${REDIRECT_URI:-http://localhost:8500/callback}"
AUTHORIZE="https://idp.xaa.dev/authorize"
TOKEN="https://idp.xaa.dev/token"
SECRET_FILE="${SECRET_FILE:-$DIR/secrets/idp-client-secret}"
CLIENT_SECRET="$(tr -d '\n' < "$SECRET_FILE")"

[ "$CLIENT_SECRET" = "PASTE_IDP_CLIENT_SECRET_HERE" ] && { echo "ERROR: put the real IdP client secret in $SECRET_FILE first"; exit 1; }

# PKCE
verifier=$(openssl rand -base64 60 | tr -d '\n=+/' | cut -c1-64)
challenge=$(printf '%s' "$verifier" | openssl dgst -binary -sha256 | openssl base64 | tr '+/' '-_' | tr -d '=\n')
state=$(openssl rand -hex 8)

url="${AUTHORIZE}?response_type=code&scope=openid%20email&redirect_uri=$(python3 -c 'import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=""))' "$REDIRECT_URI")&client_id=${CLIENT_ID}&code_challenge_method=S256&code_challenge=${challenge}&state=${state}"

echo "1) Open this URL in your browser and sign in (any email works):"
echo
echo "   $url"
echo
echo "2) After login you'll be redirected to ${REDIRECT_URI}?code=...&state=..."
echo "   (the page won't load — that's expected). Copy the value of the 'code' query param."
echo
printf "Paste the code here: "
read -r CODE

RESP=$(curl -s -X POST "$TOKEN" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d grant_type=authorization_code \
  --data-urlencode "code=$CODE" \
  --data-urlencode "redirect_uri=$REDIRECT_URI" \
  --data-urlencode "code_verifier=$verifier" \
  --data-urlencode "client_id=$CLIENT_ID" \
  --data-urlencode "client_secret=$CLIENT_SECRET")

ID_TOKEN=$(echo "$RESP" | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d.get("id_token",""))' 2>/dev/null || true)
if [ -z "$ID_TOKEN" ]; then
  echo "Token exchange failed:"; echo "$RESP"; exit 1
fi
echo
echo "ID token obtained. Use it against the gateway:"
echo
echo "  export ID_TOKEN='$ID_TOKEN'"
echo "  curl -s http://localhost:3031/ -H \"Authorization: Bearer \$ID_TOKEN\""
