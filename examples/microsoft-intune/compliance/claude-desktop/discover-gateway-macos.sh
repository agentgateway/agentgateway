#!/bin/bash

# Edit this value before uploading the script to Microsoft Intune.
EXPECTED_CLAUDE_GATEWAY_URL=${EXPECTED_CLAUDE_GATEWAY_URL:-"https://llm.example.com/claude"}

# Override only when testing the script with a temporary managed-preferences
# directory. Intune-managed devices use the default system directory.
MANAGED_PREFERENCES_DIRECTORY=${MANAGED_PREFERENCES_DIRECTORY:-"/Library/Managed Preferences"}

managed_provider=""
managed_gateway_url=""
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
fi

configured=false
if [ "$managed_provider" = "gateway" ] && \
  [ "$managed_gateway_url" = "$EXPECTED_CLAUDE_GATEWAY_URL" ]; then
  configured=true
fi

# Custom compliance consumes this value. Do not print diagnostic messages or
# return a nonzero exit code merely because the discovered value is false.
printf '%s\n' "$configured"
exit 0
