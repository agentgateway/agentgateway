#!/bin/bash
# NHP Agent Setup Script for agentgateway
#
# This script registers agentgateway as an NHP Agent with the qURL API
# using the agent bootstrap endpoint. Run this once per deployment.

set -euo pipefail

# Configuration
QURL_API_URL="${QURL_API_URL:-https://api.layerv.ai}"
QURL_API_KEY="${QURL_API_KEY:-}"
AGENT_ID="${AGENT_ID:-agentgateway-$(hostname)-$(date +%s)}"
HOSTNAME="${HOSTNAME:-$(hostname)}"
VERSION="${VERSION:-1.0.0}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }

# Check prerequisites
if [[ -z "$QURL_API_KEY" ]]; then
    log_error "QURL_API_KEY environment variable is required"
    log_error "Get an API key with 'qurl:agent' scope from layerv.ai dashboard"
    exit 1
fi

if ! command -v openssl &> /dev/null; then
    log_error "openssl is required for key generation"
    exit 1
fi

if ! command -v jq &> /dev/null; then
    log_error "jq is required for JSON parsing"
    exit 1
fi

log_info "Setting up NHP Agent for agentgateway"
log_info "Agent ID: $AGENT_ID"
log_info "Hostname: $HOSTNAME"
log_info "qURL API: $QURL_API_URL"

# Generate X25519 key pair for NHP
log_info "Generating X25519 key pair..."

# Generate private key
PRIVATE_KEY=$(openssl genpkey -algorithm X25519 -outform DER | base64 -w 0)
# Derive public key from private key
PUBLIC_KEY=$(echo "$PRIVATE_KEY" | base64 -d | openssl pkey -inform DER -pubout -outform DER | base64 -w 0)

log_info "Keys generated successfully"

# Register with qURL API
log_info "Registering agent with qURL API..."

RESPONSE=$(curl -s -X POST "${QURL_API_URL}/v1/agent/bootstrap" \
    -H "Authorization: Bearer ${QURL_API_KEY}" \
    -H "Content-Type: application/json" \
    -d "$(jq -n \
        --arg pk "$PUBLIC_KEY" \
        --arg aid "$AGENT_ID" \
        --arg host "$HOSTNAME" \
        --arg ver "$VERSION" \
        '{public_key: $pk, agent_id: $aid, hostname: $host, version: $ver}')")

# Check for errors
ERROR=$(echo "$RESPONSE" | jq -r '.error // empty')
if [[ -n "$ERROR" ]]; log_error "Registration failed: $ERROR"; echo "$RESPONSE" | jq .; exit 1; fi

# Extract registration details
AGENT_ID=$(echo "$RESPONSE" | jq -r '.data.agent_id')
REGISTERED_AT=$(echo "$RESPONSE" | jq -r '.data.registered_at')
NHP_HOST=$(echo "$RESPONSE" | jq -r '.data.nhp_server_peer.host')
NHP_PORT=$(echo "$RESPONSE" | jq -r '.data.nhp_server_peer.port')
NHP_PUBLIC_KEY=$(echo "$RESPONSE" | jq -r '.data.nhp_server_peer.public_key_b64')
NHP_EXPIRE=$(echo "$RESPONSE" | jq -r '.data.nhp_server_peer.expire_time')

log_info "Agent registered successfully!"
log_info "Agent ID: $AGENT_ID"
log_info "Registered at: $REGISTERED_AT"
log_info "NHP Server: $NHP_HOST:$NHP_PORT"
log_info "NHP Server Public Key: $NHP_PUBLIC_KEY"
log_info "NHP Peer Expires: $NHP_EXPIRE"

# Output configuration for agentgateway
cat << EOF

# ===========================================
# Add this to your agentgateway config.yaml:
# ===========================================

# In your qurlNHP provider config:
nhp_agent_id: "$AGENT_ID"

# For the NHP Agent (if running standalone NHP Agent):
# Save these to a config file or environment:

export NHP_AGENT_ID="$AGENT_ID"
export NHP_AGENT_PRIVATE_KEY="$PRIVATE_KEY"
export NHP_SERVER_HOST="$NHP_HOST"
export NHP_SERVER_PORT="$NHP_PORT"
export NHP_SERVER_PUBLIC_KEY="$NHP_PUBLIC_KEY"

# Example NHP Agent config (for nhp-agent binary):
# agent_id: "$AGENT_ID"
# private_key: "$PRIVATE_KEY"  # base64
# server:
#   host: "$NHP_HOST"
#   port: $NHP_PORT
#   public_key: "$NHP_PUBLIC_KEY"  # base64

EOF

# Save credentials to file (optional)
CREDS_FILE="/etc/agentgateway/nhp-agent-credentials.env"
if [[ -w "$(dirname "$CREDS_FILE")" ]] || [[ ! -e "$CREDS_FILE" ]]; then
    log_info "Saving credentials to $CREDS_FILE"
    cat > "$CREDS_FILE" << EOF
# NHP Agent Credentials for agentgateway
# Generated: $(date -u +"%Y-%m-%dT%H:%M:%SZ")
# Agent ID: $AGENT_ID

NHP_AGENT_ID="$AGENT_ID"
NHP_AGENT_PRIVATE_KEY="$PRIVATE_KEY"
NHP_SERVER_HOST="$NHP_HOST"
NHP_SERVER_PORT="$NHP_PORT"
NHP_SERVER_PUBLIC_KEY="$NHP_PUBLIC_KEY"
NHP_PEER_EXPIRE="$NHP_EXPIRE"
EOF
    chmod 600 "$CREDS_FILE"
fi

log_info "Setup complete! Configure agentgateway with the above details."