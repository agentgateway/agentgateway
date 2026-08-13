#!/bin/sh

# Edit these values before uploading the script to Microsoft Intune.
EXPECTED_CODEX_BASE_URL="https://llm.example.com/v1"
EXPECTED_CLAUDE_GATEWAY_URL="https://llm.example.com/claude"
VERIFY_CODEX=true
VERIFY_CLAUDE_DESKTOP=true
VERIFY_INSTALLATION=true
VERIFY_NETWORK=true

failures=0

pass() {
  printf 'PASS: %s\n' "$1"
}

fail() {
  printf 'FAIL: %s\n' "$1"
  failures=$((failures + 1))
}

is_enabled() {
  [ "$1" = "true" ]
}

verify_codex() {
  if is_enabled "$VERIFY_INSTALLATION"; then
    if [ -d "/Applications/Codex.app" ] || \
      [ -d "$HOME/Applications/Codex.app" ] || \
      command -v codex >/dev/null 2>&1 || \
      [ -x "/opt/homebrew/bin/codex" ] || \
      [ -x "/usr/local/bin/codex" ] || \
      [ -x "$HOME/.local/bin/codex" ]; then
      pass "Codex is installed."
    else
      fail "Codex is not installed in a recognized location."
    fi
  fi

  encoded_config=$(defaults read com.openai.codex config_toml_base64 2>/dev/null)
  if [ -z "$encoded_config" ]; then
    fail "Codex managed configuration is missing."
    return
  fi

  managed_config=$(printf '%s' "$encoded_config" | base64 -D 2>/dev/null)
  if [ -z "$managed_config" ]; then
    fail "Codex managed configuration is not valid base64-encoded TOML."
    return
  fi

  if printf '%s\n' "$managed_config" | \
      grep -Eq '^[[:space:]]*model_provider[[:space:]]*=[[:space:]]*"agentgateway"[[:space:]]*$' && \
    printf '%s\n' "$managed_config" | \
      grep -Fq '[model_providers.agentgateway]' && \
    printf '%s\n' "$managed_config" | \
      grep -Fq "base_url = \"$EXPECTED_CODEX_BASE_URL\"" && \
    printf '%s\n' "$managed_config" | \
      grep -Eq '^[[:space:]]*wire_api[[:space:]]*=[[:space:]]*"responses"[[:space:]]*$'; then
    pass "Codex managed configuration uses the approved agentgateway URL."
  else
    fail "Codex managed configuration does not match the approved provider, URL, and wire API."
  fi
}

verify_claude_desktop() {
  if is_enabled "$VERIFY_INSTALLATION"; then
    if [ -d "/Applications/Claude.app" ] || \
      [ -d "$HOME/Applications/Claude.app" ]; then
      pass "Claude Desktop is installed."
    else
      fail "Claude Desktop is not installed in a recognized location."
    fi
  fi

  managed_preferences=$(defaults read com.anthropic.claudefordesktop 2>/dev/null)
  if [ -z "$managed_preferences" ]; then
    fail "Claude Desktop managed preferences are missing."
    return
  fi

  if printf '%s\n' "$managed_preferences" | \
      grep -Fq "$EXPECTED_CLAUDE_GATEWAY_URL"; then
    pass "Claude Desktop managed configuration uses the approved agentgateway URL."
  else
    fail "Claude Desktop managed configuration does not contain the approved agentgateway URL."
  fi
}

verify_reachability() {
  label=$1
  url=$2

  status=$(curl --silent --show-error --output /dev/null \
    --write-out '%{http_code}' --connect-timeout 10 --max-time 15 \
    "$url" 2>/dev/null)

  case "$status" in
    [1-5][0-9][0-9])
      pass "$label received HTTP $status from the approved agentgateway URL."
      ;;
    *)
      fail "$label could not reach the approved agentgateway URL."
      ;;
  esac
}

if is_enabled "$VERIFY_CODEX"; then
  verify_codex
  if is_enabled "$VERIFY_NETWORK"; then
    verify_reachability "Codex" "$EXPECTED_CODEX_BASE_URL"
  fi
fi

if is_enabled "$VERIFY_CLAUDE_DESKTOP"; then
  verify_claude_desktop
  if is_enabled "$VERIFY_NETWORK"; then
    verify_reachability "Claude Desktop" "$EXPECTED_CLAUDE_GATEWAY_URL"
  fi
fi

if [ "$failures" -gt 0 ]; then
  printf 'Verification failed with %s failed check(s).\n' "$failures"
  exit 1
fi

printf 'All enabled agentgateway client checks passed.\n'
exit 0
