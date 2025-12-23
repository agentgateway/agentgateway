# Easy Auth Deployment Configuration

This document captures the Azure Container Apps Easy Auth configuration for OAuth authentication with Microsoft and Google providers.

## Infrastructure Configuration

The Easy Auth configuration is applied via Azure CLI. This is **infrastructure-level** authentication managed by Azure Container Apps, not application-level code.

### Current Configuration

**Container App:** `unitone-agentgateway`
**Resource Group:** `mcp-gateway-dev-rg`
**App URL:** `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io`

### OAuth Providers

#### Microsoft (Azure AD)
- **Client ID:** `4b497d98-cb3a-400e-9374-0e23d57dd480`
- **Tenant ID:** `2cbdd19f-2624-461f-9901-bd63473655a7`
- **OpenID Issuer:** `https://login.microsoftonline.com/2cbdd19f-2624-461f-9901-bd63473655a7/v2.0`
- **Allowed Audiences:** `api://4b497d98-cb3a-400e-9374-0e23d57dd480`
- **Client Secret:** Stored in Container App secret: `unitone-gateway-client-secret`
- **Redirect URI:** `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/aad/callback`

#### Google OAuth 2.0
- **Client ID:** `919355621898-us1vie0rv5mqaff752hhqb9espne87ug.apps.googleusercontent.com`
- **Client Secret:** Stored in Container App secret: `google-client-secret`
- **Redirect URI:** `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/google/callback`

### Authentication Behavior

- **Unauthenticated Action:** Require authentication for UI routes
- **Endpoints:**
  - `/.auth/me` - Returns authenticated user information
  - `/.auth/login/aad` - Microsoft login
  - `/.auth/login/google` - Google login
  - `/.auth/logout` - Logout and clear session

## Deployment Steps

### 1. Configure Microsoft OAuth Provider

```bash
az containerapp auth microsoft update \
  --name unitone-agentgateway \
  --resource-group mcp-gateway-dev-rg \
  --client-id "4b497d98-cb3a-400e-9374-0e23d57dd480" \
  --client-secret-name "unitone-gateway-client-secret" \
  --issuer "https://login.microsoftonline.com/2cbdd19f-2624-461f-9901-bd63473655a7/v2.0" \
  --allowed-audiences "api://4b497d98-cb3a-400e-9374-0e23d57dd480" \
  --yes
```

### 2. Configure Google OAuth Provider

```bash
az containerapp auth update \
  --name unitone-agentgateway \
  --resource-group mcp-gateway-dev-rg \
  --set identityProviders.google.registration.clientId="919355621898-us1vie0rv5mqaff752hhqb9espne87ug.apps.googleusercontent.com" \
  --set identityProviders.google.registration.clientSecretSettingName="google-client-secret"
```

### 3. Set Client Secrets

```bash
# Set Microsoft client secret
az containerapp secret set \
  --name unitone-agentgateway \
  --resource-group mcp-gateway-dev-rg \
  --secrets unitone-gateway-client-secret="<MICROSOFT_CLIENT_SECRET>"

# Set Google client secret
az containerapp secret set \
  --name unitone-agentgateway \
  --resource-group mcp-gateway-dev-rg \
  --secrets google-client-secret="<GOOGLE_CLIENT_SECRET>"
```

## Azure Portal Configuration

### Microsoft App Registration (UNITONE Gateway)
1. **App ID:** `4b497d98-cb3a-400e-9374-0e23d57dd480`
2. **Redirect URIs:**
   - `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/aad/callback`
3. **Expose an API:**
   - Application ID URI: `api://4b497d98-cb3a-400e-9374-0e23d57dd480`
   - Scopes: `read`, `write`
4. **Token Configuration:**
   - Optional claims: `email`, `preferred_username`, `name`

### Google Cloud Console
1. **Project:** Your Google Cloud Project
2. **OAuth 2.0 Client ID:** `919355621898-us1vie0rv5mqaff752hhqb9espne87ug.apps.googleusercontent.com`
3. **Authorized redirect URIs:**
   - `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/google/callback`

## Testing

1. **Logout:** `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/logout`
2. **Access UI:** `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/ui`
3. **Check Auth:** `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/me`

## Notes

- Easy Auth is **infrastructure-level** authentication, not part of the application code
- The application doesn't handle OAuth flows directly
- All OAuth redirects and token management is handled by Azure Container Apps Easy Auth
- The UI application only needs to fetch user info from `/.auth/me`
- This configuration is **NOT** stored in source code for security reasons
- Secrets are managed separately in Azure Key Vault or Container App secrets
