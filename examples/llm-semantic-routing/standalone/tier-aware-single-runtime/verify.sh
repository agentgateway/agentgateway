#!/usr/bin/env bash
set -euo pipefail

ENDPOINT=${ENDPOINT:-http://127.0.0.1:4000}
RUN_LIVE_PROVIDER_TESTS=${RUN_LIVE_PROVIDER_TESTS:-false}
for tool in curl jq; do
  command -v "$tool" >/dev/null || { echo "Required command not found: $tool" >&2; exit 1; }
done
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

request() {
  local expected=$1 tier=$2 model=$3 prompt=$4
  shift 4
  local status
  status=$(curl --silent --show-error --max-time 120 \
    -D "$TMP_DIR/headers" -o "$TMP_DIR/body" -w '%{http_code}' \
    "$ENDPOINT/v1/chat/completions" \
    -H 'Content-Type: application/json' -H "X-Authz-User-Id: ${USER_ID-demo-user}" \
    -H "X-Entitlement-Tier: $tier" -H 'X-VSR-Debug: true' \
    -d "$(jq -nc --arg model "$model" --arg prompt "$prompt" \
      '{model:$model,messages:[{role:"user",content:$prompt}],max_tokens:64}')" "$@")
  if [[ "$status" != "$expected" ]]; then
    echo "FAIL: tier=$tier model=$model: expected HTTP $expected, got $status" >&2
    cat "$TMP_DIR/body" >&2
    exit 1
  fi
  echo "PASS: tier=$tier model=$model HTTP $status"
}

selected() {
  local model=$1 decision=${2:-}
  tr -d '\r' < "$TMP_DIR/headers" | grep -Fxi "x-vsr-selected-model: $model" >/dev/null
  if [[ -n "$decision" ]]; then
    tr -d '\r' < "$TMP_DIR/headers" | grep -Fxi "x-vsr-selected-decision: $decision" >/dev/null
  fi
  jq -e '.choices[0].message.content | type == "string" and length > 0' "$TMP_DIR/body" >/dev/null
}

request 403 basic claude-haiku-4-5-20251001 'Say hello.'
request 403 basic claude-sonnet-4-6 'Say hello.'
request 403 standard claude-sonnet-4-6 'Say hello.'
request 403 '' auto 'Say hello.'
request 403 unknown auto 'Say hello.'
USER_ID='' request 403 basic auto 'Say hello.'
request 403 basic auto 'Say hello.' -H 'X-VSR-Skip-Processing: true'
request 400 basic unconfigured-model 'Say hello.'

if [[ "$RUN_LIVE_PROVIDER_TESTS" != true ]]; then
  echo 'Provider tests skipped. Set RUN_LIVE_PROVIDER_TESTS=true to make billable provider calls.'
  exit 0
fi

for tier in basic standard pro; do
  case "$tier" in
    basic) model=gpt-5.4 ;;
    standard) model=claude-haiku-4-5-20251001 ;;
    pro) model=claude-sonnet-4-6 ;;
  esac
  request 200 "$tier" auto 'Define quantum physics in one sentence.'
  selected "$model" "${tier}_stem"
  request 200 "$tier" auto 'Say hello.'
  selected gpt-4.1
done
request 200 standard claude-haiku-4-5-20251001 'Say hello.'
jq -e '.choices[0].message.content | length > 0' "$TMP_DIR/body" >/dev/null

curl --fail-with-body --silent --show-error --no-buffer --max-time 120 \
  "$ENDPOINT/v1/chat/completions" \
  -H 'Content-Type: application/json' -H "X-Authz-User-Id: ${USER_ID-demo-user}" \
  -H 'X-Entitlement-Tier: pro' -H 'X-VSR-Debug: true' \
  -D "$TMP_DIR/headers" -o "$TMP_DIR/body" \
  -d '{"model":"auto","messages":[{"role":"user","content":"Define quantum physics in one sentence."}],"max_tokens":64,"stream":true}'
tr -d '\r' < "$TMP_DIR/headers" | grep -Fxi 'x-vsr-selected-model: claude-sonnet-4-6' >/dev/null
grep -F 'data: [DONE]' "$TMP_DIR/body" >/dev/null
sed -n 's/^data: //p' "$TMP_DIR/body" | sed '/^\[DONE\]/d' | \
  jq -se 'any(.[]; (.choices[0].delta.content // "") | length > 0)' >/dev/null
echo 'PASS: streaming response with generated text'
