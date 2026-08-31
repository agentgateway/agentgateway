#!/bin/bash

# Set these defaults to match the Intune-managed Claude Desktop profile before
# uploading the script. Intune does not provide custom environment variables
# to macOS discovery scripts; the environment overrides are for local testing.

# Exact inferenceGatewayBaseUrl. Include a route prefix, such as /claude, only
# when it is part of the managed Gateway URL.
EXPECTED_CLAUDE_GATEWAY_URL=${EXPECTED_CLAUDE_GATEWAY_URL:-"https://llm.example.com/claude"}

# Use static for Gateway API key authentication or interactive for Entra ID.
EXPECTED_CLAUDE_CREDENTIAL_KIND=${EXPECTED_CLAUDE_CREDENTIAL_KIND:-"static"}

# Required only for interactive authentication. The flow is browser or broker.
EXPECTED_CLAUDE_OIDC_AUTH_FLOW=${EXPECTED_CLAUDE_OIDC_AUTH_FLOW:-""}
EXPECTED_CLAUDE_OIDC_ISSUER=${EXPECTED_CLAUDE_OIDC_ISSUER:-""}
EXPECTED_CLAUDE_OIDC_CLIENT_ID=${EXPECTED_CLAUDE_OIDC_CLIENT_ID:-""}

# Override only when testing the script with a temporary managed-preferences
# directory. Intune-managed devices use the default system directory.
MANAGED_PREFERENCES_DIRECTORY=${MANAGED_PREFERENCES_DIRECTORY:-"/Library/Managed Preferences"}

managed_provider=""
managed_gateway_url=""
managed_credential_kind=""
managed_oidc_auth_flow=""
managed_oidc=""
managed_api_key=""
managed_user=$(stat -f '%Su' /dev/console 2>/dev/null)
case "$managed_user" in
  ""|root|loginwindow)
    managed_user=$(id -un 2>/dev/null) || exit 1
    ;;
esac

for preference_file in \
  "$MANAGED_PREFERENCES_DIRECTORY/$managed_user/com.anthropic.claudefordesktop.plist" \
  "$MANAGED_PREFERENCES_DIRECTORY/com.anthropic.claudefordesktop.plist"
do
  if [ -r "$preference_file" ]; then
    managed_provider=$(plutil -extract inferenceProvider raw \
      -expect string "$preference_file" 2>/dev/null)
    managed_gateway_url=$(plutil -extract inferenceGatewayBaseUrl raw \
      -expect string "$preference_file" 2>/dev/null)
    managed_credential_kind=$(plutil -extract inferenceCredentialKind raw \
      -expect string "$preference_file" 2>/dev/null)
    managed_oidc_auth_flow=$(plutil -extract inferenceGatewayOidcAuthFlow raw \
      -expect string "$preference_file" 2>/dev/null)
    managed_oidc=$(plutil -extract inferenceGatewayOidc raw \
      -expect string "$preference_file" 2>/dev/null)
    managed_api_key=$(plutil -extract inferenceGatewayApiKey raw \
      -expect string "$preference_file" 2>/dev/null)
    if [ -n "$managed_provider" ] && [ -n "$managed_gateway_url" ]; then
      break
    fi
  fi
done

# Fall back to the effective user preference domain for profiles that expose
# their managed values only through the effective preference search path.
if [ -z "$managed_provider" ] || [ -z "$managed_gateway_url" ]; then
  managed_provider=$(defaults read com.anthropic.claudefordesktop \
    inferenceProvider 2>/dev/null)
  managed_gateway_url=$(defaults read com.anthropic.claudefordesktop \
    inferenceGatewayBaseUrl 2>/dev/null)
  managed_credential_kind=$(defaults read com.anthropic.claudefordesktop \
    inferenceCredentialKind 2>/dev/null)
  managed_oidc_auth_flow=$(defaults read com.anthropic.claudefordesktop \
    inferenceGatewayOidcAuthFlow 2>/dev/null)
  managed_oidc=$(defaults read com.anthropic.claudefordesktop \
    inferenceGatewayOidc 2>/dev/null)
  managed_api_key=$(defaults read com.anthropic.claudefordesktop \
    inferenceGatewayApiKey 2>/dev/null)
fi

configured=false
if [ "$managed_provider" = "gateway" ] && \
  [ "$managed_gateway_url" = "$EXPECTED_CLAUDE_GATEWAY_URL" ] && \
  [ "$managed_credential_kind" = "$EXPECTED_CLAUDE_CREDENTIAL_KIND" ]; then
  case "$EXPECTED_CLAUDE_CREDENTIAL_KIND" in
    static)
      if [ -n "$managed_api_key" ]; then
        configured=true
      fi
      ;;
    interactive)
      compact_oidc=$(printf '%s' "$managed_oidc" | tr -d '[:space:]')
      if [ -n "$EXPECTED_CLAUDE_OIDC_AUTH_FLOW" ] && \
        [ -n "$EXPECTED_CLAUDE_OIDC_ISSUER" ] && \
        [ -n "$EXPECTED_CLAUDE_OIDC_CLIENT_ID" ] && \
        [ "$managed_oidc_auth_flow" = "$EXPECTED_CLAUDE_OIDC_AUTH_FLOW" ] && \
        printf '%s' "$compact_oidc" | \
          grep -Fq "\"issuer\":\"$EXPECTED_CLAUDE_OIDC_ISSUER\"" && \
        printf '%s' "$compact_oidc" | \
          grep -Fq "\"clientId\":\"$EXPECTED_CLAUDE_OIDC_CLIENT_ID\"" && \
        printf '%s' "$compact_oidc" | \
          grep -Fq '"bearerTokenType":"id_token"' && \
        [ -z "$managed_api_key" ]; then
        configured=true
      fi
      ;;
    *)
      configured=false
      ;;
  esac
fi

# Custom compliance consumes this JSON object. Do not print diagnostic
# messages or return a nonzero exit code merely because the discovered value
# is false.
printf '{"ClaudeDesktopGatewayConfigured":%s}\n' "$configured"
exit 0
