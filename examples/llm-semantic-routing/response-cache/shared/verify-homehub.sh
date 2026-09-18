#!/usr/bin/env bash
# Sourced by the deployment-specific verification scripts.

cache_header() {
  awk 'tolower($1) == "x-vsr-cache-hit:" {gsub("\r", "", $2); print tolower($2)}' "$1" | tail -n 1
}

decision_header() {
  awk 'tolower($1) == "x-vsr-selected-decision:" {gsub("\r", "", $2); print $2}' "$1" | tail -n 1
}

send_request() {
  local name=$1
  local prompt=$2
  jq -n --arg prompt "${prompt}" '{
    model: "auto",
    stream: false,
    messages: [{role: "user", content: $prompt}],
    max_tokens: 96
  }' >"${WORK_DIR}/${name}-request.json"
  curl -fsS -D "${WORK_DIR}/${name}-headers.txt" \
    -o "${WORK_DIR}/${name}-body.json" \
    "${GATEWAY_URL}/v1/chat/completions" \
    -H 'Content-Type: application/json' \
    -H 'X-VSR-Debug: true' \
    -H "X-Request-ID: semantic-cache-${name}" \
    --data-binary "@${WORK_DIR}/${name}-request.json"
  jq -e '.choices[0].message.content | type == "string"' \
    "${WORK_DIR}/${name}-body.json" >/dev/null
}

assert_count() {
  local expected=$1
  local actual
  actual=$(backend_count)
  if [[ "${actual}" != "${expected}" ]]; then
    echo "expected backend count ${expected}, got ${actual}" >&2
    exit 1
  fi
}

assert_hit() {
  local name=$1
  local hit
  hit=$(cache_header "${WORK_DIR}/${name}-headers.txt")
  if [[ "${hit}" != true ]]; then
    echo "expected ${name} to be a cache hit" >&2
    sed -n '1,40p' "${WORK_DIR}/${name}-headers.txt" >&2
    exit 1
  fi
}

assert_uncached_decision() {
  local name=$1
  local decision
  decision=$(decision_header "${WORK_DIR}/${name}-headers.txt")
  if [[ "${decision}" != uncached_homehub_support ]]; then
    echo "expected ${name} to select uncached_homehub_support, got ${decision}" >&2
    exit 1
  fi
}

verify_requests() {
  echo "Verifying initial miss"
  send_request warm "How do I factory-reset my HomeHub X2?"
  assert_count 1

  echo "Verifying exact hit"
  send_request exact "How do I factory-reset my HomeHub X2?"
  assert_hit exact
  assert_count 1

  echo "Verifying paraphrase hit"
  send_request paraphrase "What is the procedure for restoring a HomeHub X2 to factory settings?"
  assert_hit paraphrase
  assert_count 1

  echo "Verifying semantically different miss"
  send_request different "Can a HomeHub be used outdoors in freezing rain?"
  assert_uncached_decision different
  assert_count 2

}

# Redis is dedicated to this example. Current vSR derives physical indexes
# and key prefixes from the embedding identity; older builds use the config.
reset_cache() {
  redis_cli EVAL \
    "local n=0; for _,p in ipairs(ARGV) do local k=redis.call('keys',p); for _,v in ipairs(k) do n=n+redis.call('del',v) end end; return n" \
    0 'semantic-cache:*' 'vsr:embedding-cache:*' >/dev/null
}

check_redis_indexes() {
  local indexes index
  indexes=$(redis_cli --raw FT._LIST | awk '/^semantic_cache_/')
  if [[ -z "${indexes}" ]]; then
    echo "no response-cache Redis Search index found" >&2
    return 1
  fi
  while IFS= read -r index; do
    redis_cli FT.INFO "${index}" >/dev/null
  done <<< "${indexes}"
}
