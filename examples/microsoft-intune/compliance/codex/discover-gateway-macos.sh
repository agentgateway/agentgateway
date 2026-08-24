#!/bin/bash

# Set these defaults to match the Intune-managed Codex TOML before uploading
# the script. Intune does not provide custom environment variables to macOS
# discovery scripts; the environment overrides are for local testing.

# Use the exact base_url, including /v1, and the matching env_key name.
EXPECTED_CODEX_BASE_URL=${EXPECTED_CODEX_BASE_URL:-"https://llm.example.com/v1"}
EXPECTED_CODEX_ENV_KEY=${EXPECTED_CODEX_ENV_KEY:-"AGENTGATEWAY_API_KEY"}

# Override only when testing the script with a temporary managed-preferences
# directory. Intune-managed devices use the default system directory.
MANAGED_PREFERENCES_DIRECTORY=${MANAGED_PREFERENCES_DIRECTORY:-"/Library/Managed Preferences"}

encoded_config=""
managed_config=""
managed_user=$(stat -f '%Su' /dev/console 2>/dev/null)
case "$managed_user" in
  ""|root|loginwindow)
    managed_user=$(id -un 2>/dev/null) || exit 1
    ;;
esac

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

if [ -n "$encoded_config" ]; then
  managed_config=$(printf '%s' "$encoded_config" | base64 -D 2>/dev/null)
fi

configured=false
if [ -n "$managed_config" ] && \
  printf '%s\n' "$managed_config" | \
    grep -Eq '^[[:space:]]*model_provider[[:space:]]*=[[:space:]]*"agentgateway"[[:space:]]*$' && \
  printf '%s\n' "$managed_config" | \
    grep -Fq '[model_providers.agentgateway]' && \
  printf '%s\n' "$managed_config" | \
    grep -Fq "base_url = \"$EXPECTED_CODEX_BASE_URL\"" && \
  printf '%s\n' "$managed_config" | \
    grep -Eq '^[[:space:]]*wire_api[[:space:]]*=[[:space:]]*"responses"[[:space:]]*$' && \
  printf '%s\n' "$managed_config" | \
    grep -Eq "^[[:space:]]*env_key[[:space:]]*=[[:space:]]*\"$EXPECTED_CODEX_ENV_KEY\"[[:space:]]*$"; then
  configured=true
fi

# Custom compliance consumes this JSON object. Do not print diagnostic
# messages or return a nonzero exit code merely because the discovered value
# is false.
printf '{"CodexGatewayConfigured":%s}\n' "$configured"
exit 0
