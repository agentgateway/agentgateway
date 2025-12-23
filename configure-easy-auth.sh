#!/bin/bash
#
# Easy Auth Configuration Script for AgentGateway
# Fill in your OAuth credentials below and run this script
#

set -e

echo "========================================="
echo "Easy Auth Configuration for AgentGateway"
echo "========================================="

# ============================================
# FILL IN YOUR OAuth CREDENTIALS HERE
# ============================================

# Microsoft/Azure AD OAuth
MICROSOFT_CLIENT_ID="YOUR_MICROSOFT_CLIENT_ID_HERE"
MICROSOFT_CLIENT_SECRET="YOUR_MICROSOFT_CLIENT_SECRET_HERE"
MICROSOFT_TENANT_ID="common"  # Use 'common' for multi-tenant, or your specific tenant ID

# Google OAuth
GOOGLE_CLIENT_ID="YOUR_GOOGLE_CLIENT_ID_HERE"
GOOGLE_CLIENT_SECRET="YOUR_GOOGLE_CLIENT_SECRET_HERE"

# Container App Details
CONTAINER_APP_NAME="unitone-agentgateway"
RESOURCE_GROUP="mcp-gateway-dev-rg"
APP_URL="https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io"

# ============================================
# Validation
# ============================================

if [[ "$MICROSOFT_CLIENT_ID" == "YOUR_"* ]] || [[ "$GOOGLE_CLIENT_ID" == "YOUR_"* ]]; then
    echo "❌ ERROR: Please fill in your OAuth credentials in this script first!"
    echo ""
    echo "Edit this file and replace:"
    echo "  - MICROSOFT_CLIENT_ID"
    echo "  - MICROSOFT_CLIENT_SECRET"
    echo "  - GOOGLE_CLIENT_ID"
    echo "  - GOOGLE_CLIENT_SECRET"
    echo ""
    exit 1
fi

echo "✅ Credentials provided"
echo ""

# ============================================
# Step 1: Configure Microsoft Identity Provider
# ============================================

echo "Step 1: Configuring Microsoft identity provider..."

az containerapp auth microsoft update \
  --name "$CONTAINER_APP_NAME" \
  --resource-group "$RESOURCE_GROUP" \
  --client-id "$MICROSOFT_CLIENT_ID" \
  --client-secret "$MICROSOFT_CLIENT_SECRET" \
  --tenant-id "$MICROSOFT_TENANT_ID" \
  --allowed-audiences "api://unitone-agentgateway" \
  --yes

echo "✅ Microsoft identity provider configured"
echo ""

# ============================================
# Step 2: Configure Google Identity Provider
# ============================================

echo "Step 2: Configuring Google identity provider..."

# Note: Azure CLI doesn't have direct Google provider support via command line
# We need to use the Azure REST API or Portal for Google
echo "⚠️  Google provider configuration via CLI requires REST API"
echo ""
echo "To add Google provider, you have two options:"
echo ""
echo "Option A: Use Azure Portal"
echo "  1. Go to: https://portal.azure.com"
echo "  2. Navigate to: Resource Groups > $RESOURCE_GROUP > $CONTAINER_APP_NAME"
echo "  3. Click 'Authentication' in the left menu"
echo "  4. Click 'Add identity provider'"
echo "  5. Select 'Google'"
echo "  6. Enter Client ID: $GOOGLE_CLIENT_ID"
echo "  7. Enter Client Secret: $GOOGLE_CLIENT_SECRET"
echo "  8. Click 'Add'"
echo ""
echo "Option B: Use REST API (requires jq)"
if command -v jq &> /dev/null; then
    echo "  jq found, attempting REST API configuration..."

    # Get subscription ID
    SUBSCRIPTION_ID=$(az account show --query id -o tsv)

    # Get auth config
    AUTH_CONFIG_ID="/subscriptions/$SUBSCRIPTION_ID/resourceGroups/$RESOURCE_GROUP/providers/Microsoft.App/containerApps/$CONTAINER_APP_NAME/authConfigs/current"

    # Update auth config with Google provider
    echo "  Adding Google provider via REST API..."

    az rest --method patch \
      --uri "https://management.azure.com${AUTH_CONFIG_ID}?api-version=2023-05-01" \
      --body "{
        \"properties\": {
          \"identityProviders\": {
            \"google\": {
              \"enabled\": true,
              \"registration\": {
                \"clientId\": \"$GOOGLE_CLIENT_ID\",
                \"clientSecretSettingName\": \"google-client-secret\"
              }
            }
          }
        }
      }" || echo "  REST API configuration failed, please use Azure Portal"

    # Add Google client secret as Container App secret
    echo "  Adding Google client secret..."
    az containerapp secret set \
      --name "$CONTAINER_APP_NAME" \
      --resource-group "$RESOURCE_GROUP" \
      --secrets google-client-secret="$GOOGLE_CLIENT_SECRET" || echo "  Failed to set secret"

    echo "✅ Google provider configured via REST API"
else
    echo "  jq not found, please use Azure Portal (Option A above)"
fi

echo ""

# ============================================
# Step 3: Update Authentication Settings
# ============================================

echo "Step 3: Updating authentication settings..."

# Allow unauthenticated access (so API endpoints with tokens still work)
az containerapp auth update \
  --name "$CONTAINER_APP_NAME" \
  --resource-group "$RESOURCE_GROUP" \
  --unauthenticated-client-action AllowAnonymous \
  --yes

echo "✅ Authentication settings updated"
echo ""

# ============================================
# Step 4: Add Redirect URIs to Your OAuth Apps
# ============================================

echo "========================================="
echo "IMPORTANT: Update Redirect URIs"
echo "========================================="
echo ""
echo "Add these redirect URIs to your OAuth applications:"
echo ""
echo "Microsoft/Azure AD:"
echo "  ${APP_URL}/.auth/login/aad/callback"
echo ""
echo "Google:"
echo "  ${APP_URL}/.auth/login/google/callback"
echo ""
echo "========================================="
echo ""

# ============================================
# Step 5: Test the Configuration
# ============================================

echo "Step 5: Testing configuration..."
echo ""
echo "Your AgentGateway is now configured with Easy Auth!"
echo ""
echo "Test URLs:"
echo "  Main URL: $APP_URL"
echo "  Login endpoint: ${APP_URL}/.auth/login/aad"
echo "  Google login: ${APP_URL}/.auth/login/google"
echo ""
echo "Visit the URL in your browser to test web login."
echo ""

echo "✅ Configuration complete!"
