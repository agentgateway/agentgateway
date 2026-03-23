#!/usr/bin/env bash
set -euo pipefail

# Prompt for sudo password immediately and keep refreshing in background
sudo -v
while true; do sudo -n true; sleep 60; done 2>/dev/null &
SUDO_REFRESH_PID=$!
trap 'kill "$SUDO_REFRESH_PID" 2>/dev/null' EXIT

# Build the UI.
BASE_PATH=/ui/ yarn --cwd=ui build

# Make sure that the schemas are up to date.
make generate-schema

# Build and install binary
make build
sudo install -m 755 ./target/release/agentgateway /usr/local/bin/agentgateway

# Print installed version
agentgateway --version