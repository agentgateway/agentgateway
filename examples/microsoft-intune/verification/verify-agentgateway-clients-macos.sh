#!/bin/sh

# Edit these values before uploading the script to Microsoft Intune.
EXPECTED_CODEX_BASE_URL=${EXPECTED_CODEX_BASE_URL:-"https://llm.example.com/v1"}
EXPECTED_CLAUDE_GATEWAY_URL=${EXPECTED_CLAUDE_GATEWAY_URL:-"https://llm.example.com/claude"}
VERIFY_CODEX=${VERIFY_CODEX:-true}
VERIFY_CLAUDE_DESKTOP=${VERIFY_CLAUDE_DESKTOP:-true}
VERIFY_INSTALLATION=${VERIFY_INSTALLATION:-true}
VERIFY_NETWORK=${VERIFY_NETWORK:-true}
DEFAULT_LOG_FILE="$HOME/Library/Logs/agentgateway/intune-verification.log"
LOG_FILE=${AGENTGATEWAY_INTUNE_LOG_FILE:-"$DEFAULT_LOG_FILE"}

# Override only when testing the script with a temporary managed-preferences
# directory. Intune-managed devices use the default system directory.
MANAGED_PREFERENCES_DIRECTORY=${MANAGED_PREFERENCES_DIRECTORY:-"/Library/Managed Preferences"}

failures=0

initialize_log() {
  case "$LOG_FILE" in
    /*) ;;
    *)
      printf 'FAIL: Verification log path must be absolute: %s\n' "$LOG_FILE"
      exit 1
      ;;
  esac

  log_directory=$(dirname "$LOG_FILE")
  if ! (umask 077 && mkdir -p "$log_directory" && : >"$LOG_FILE"); then
    printf 'FAIL: Cannot create verification log at %s.\n' "$LOG_FILE"
    exit 1
  fi

  if ! chmod 600 "$LOG_FILE" 2>/dev/null; then
    printf 'FAIL: Cannot secure verification log at %s.\n' "$LOG_FILE"
    exit 1
  fi
}

log_line() {
  printf '%s\n' "$1"
  if ! printf '%s\n' "$1" >>"$LOG_FILE"; then
    printf 'FAIL: Cannot write verification log at %s.\n' "$LOG_FILE"
    exit 1
  fi
}

pass() {
  log_line "PASS: $1"
}

fail() {
  log_line "FAIL: $1"
  failures=$((failures + 1))
}

is_enabled() {
  [ "$1" = "true" ]
}

initialize_log

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

  encoded_config=""
  managed_user=$(id -un)

  for preference_file in \
    "$MANAGED_PREFERENCES_DIRECTORY/$managed_user/com.openai.codex.plist" \
    "$MANAGED_PREFERENCES_DIRECTORY/com.openai.codex.plist"
  do
    if [ -r "$preference_file" ]; then
      encoded_config=$(plutil -extract config_toml_base64 raw \
        -expect string "$preference_file" 2>/dev/null)
      if [ -n "$encoded_config" ]; then
        break
      fi
    fi
  done

  # Fall back to the effective user preference domain for clients whose
  # management profile writes a user-scoped preference.
  if [ -z "$encoded_config" ]; then
    encoded_config=$(defaults read com.openai.codex \
      config_toml_base64 2>/dev/null)
  fi

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
  log_line "Verification failed with $failures failed check(s)."
  exit 1
fi

log_line "All enabled agentgateway client checks passed."
exit 0
