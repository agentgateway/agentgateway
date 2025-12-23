# UnitOne AgentGateway - Azure Deployment Guide

Complete Infrastructure-as-Code deployment for AgentGateway with OAuth support, based on MCP Scanner deployment pattern.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Azure Container App                          │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │  AgentGateway (Port 8080)                                  │ │
│  │  ┌──────────────┬──────────────┬──────────────────────┐   │ │
│  │  │ /ui          │ /mcp/*       │ /.well-known/*       │   │ │
│  │  │ (Admin UI)   │ (MCP OAuth)  │ (OAuth metadata)     │   │ │
│  │  └──────────────┴──────────────┴──────────────────────┘   │ │
│  └────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                            │
          ┌─────────────────┴─────────────────┐
          │                                    │
   ┌──────▼──────┐                    ┌───────▼────────┐
   │  Key Vault  │                    │  App Insights  │
   │  (Secrets)  │                    │  (Monitoring)  │
   └─────────────┘                    └────────────────┘
```

## Features

- **OAuth Authentication**: GitHub, Microsoft (Azure AD), Google
- **Key Vault Integration**: Secure secret management with Managed Identity
- **MCP-Native OAuth**: Built-in support for MCP Authorization spec
- **Infrastructure as Code**: Full Bicep templates for reproducible deployments
- **Multi-Environment**: Dev, Staging, Production configurations
- **Monitoring**: Application Insights integration
- **Auto-Scaling**: HTTP-based scaling rules

## Prerequisites

1. **Azure CLI** installed and logged in:
   ```bash
   az login
   az account set --subscription <YOUR_SUBSCRIPTION_ID>
   ```

2. **OAuth App Registrations** (for each provider you want to use):
   - **GitHub**: https://github.com/settings/applications/new
   - **Microsoft/Azure AD**: https://portal.azure.com → Azure AD → App registrations
   - **Google**: https://console.cloud.google.com/apis/credentials

3. **Docker** (optional, for local builds):
   ```bash
   docker --version
   ```

## Quick Start

### 1. Setup OAuth Secrets

First, create a central Key Vault for storing OAuth secrets:

```bash
# Create secrets resource group
az group create --name unitone-secrets-rg --location eastus2

# Create central secrets Key Vault
az keyvault create \
  --name unitone-secrets-kv \
  --resource-group unitone-secrets-rg \
  --location eastus2

# Store OAuth secrets (dev environment)
az keyvault secret set --vault-name unitone-secrets-kv --name agw-dev-github-client-id --value "YOUR_GITHUB_CLIENT_ID"
az keyvault secret set --vault-name unitone-secrets-kv --name agw-dev-github-client-secret --value "YOUR_GITHUB_CLIENT_SECRET"
az keyvault secret set --vault-name unitone-secrets-kv --name agw-dev-microsoft-client-id --value "YOUR_MICROSOFT_CLIENT_ID"
az keyvault secret set --vault-name unitone-secrets-kv --name agw-dev-microsoft-client-secret --value "YOUR_MICROSOFT_CLIENT_SECRET"
az keyvault secret set --vault-name unitone-secrets-kv --name agw-dev-google-client-id --value "YOUR_GOOGLE_CLIENT_ID"
az keyvault secret set --vault-name unitone-secrets-kv --name agw-dev-google-client-secret --value "YOUR_GOOGLE_CLIENT_SECRET"

# Get ACR password (if ACR already exists) or will be auto-generated
az keyvault secret set --vault-name unitone-secrets-kv --name agw-dev-acr-password --value "$(az acr credential show --name agwimages --query passwords[0].value -o tsv)"
```

### 2. Update Parameter Files

Edit `deploy/bicep/parameters-dev.json` and replace `<SUBSCRIPTION_ID>` with your Azure subscription ID:

```bash
SUBSCRIPTION_ID=$(az account show --query id -o tsv)
sed -i.bak "s|<SUBSCRIPTION_ID>|$SUBSCRIPTION_ID|g" deploy/bicep/parameters-dev.json
```

### 3. Deploy Infrastructure

```bash
cd deploy

# Deploy to dev environment (with image build)
./deploy.sh --environment dev --build --tag latest

# Or deploy without building (use existing image)
./deploy.sh --environment dev
```

### 4. Access Your Deployment

After deployment completes, you'll see:

```
===================================
Deployment Complete!
===================================
UI URL: https://unitone-agw-dev-app.....azurecontainerapps.io/ui
MCP Endpoint: https://unitone-agw-dev-app.....azurecontainerapps.io/mcp
===================================
```

## OAuth Configuration

### MCP Scanner Pattern

AgentGateway follows the same OAuth pattern as MCP Scanner:

1. **Secrets in Key Vault**: OAuth credentials stored securely
2. **Managed Identity**: Container App uses system-assigned identity to access Key Vault
3. **Environment Variables**: Secrets injected as environment variables
4. **MCP-Native Auth**: Uses `mcpAuthentication` policy for MCP endpoints

### Supported OAuth Providers

#### 1. GitHub OAuth

**Use Case**: GitHub Actions, GitHub Apps

**Endpoints**:
- MCP: `/mcp/github`
- Metadata: `/.well-known/oauth-protected-resource/mcp/github`

**Required Scopes**: `read:all`, `write:all`

#### 2. Microsoft Azure AD

**Use Case**: Enterprise SSO, Microsoft 365 integration

**Endpoints**:
- MCP: `/mcp/microsoft`
- Metadata: `/.well-known/oauth-protected-resource/mcp/microsoft`

**Required Scopes**: `api://unitone-agentgateway/read`, `api://unitone-agentgateway/write`

#### 3. Google OAuth

**Use Case**: Google Workspace, Gmail integration

**Endpoints**:
- MCP: `/mcp/google`
- Metadata: `/.well-known/oauth-protected-resource/mcp/google`

**Required Scopes**: `openid`, `profile`, `email`

### Testing OAuth Endpoints

```bash
# Test without token (should return 401)
curl -i https://your-app.azurecontainerapps.io/mcp/github

# Expected response:
# HTTP/1.1 401 Unauthorized
# WWW-Authenticate: Bearer resource_metadata="https://your-app.azurecontainerapps.io/.well-known/oauth-protected-resource/mcp/github"

# Test with valid token
curl -H "Authorization: Bearer YOUR_ACCESS_TOKEN" \
     https://your-app.azurecontainerapps.io/mcp/github
```

## Deployment Commands

### Build and Deploy

```bash
# Build new image and deploy to dev
./deploy.sh --environment dev --build --tag v1.2.3

# Deploy to staging with existing image
./deploy.sh --environment staging --tag v1.2.3

# Deploy to production (uses 'stable' tag by default)
./deploy.sh --environment prod --build --tag stable
```

### Update Configuration Only

If you only changed the config file (no code changes):

```bash
# Update the config in Docker image
az acr build \
  --registry unitoneagwdevacr \
  --image unitone-agentgateway:latest \
  --file Dockerfile.acr \
  --platform linux/amd64 \
  .

# Trigger Container App revision update
az containerapp revision copy \
  --name unitone-agw-dev-app \
  --resource-group unitone-agw-dev-rg
```

### View Logs

```bash
# Follow logs in real-time
az containerapp logs show \
  --name unitone-agw-dev-app \
  --resource-group unitone-agw-dev-rg \
  --follow

# View last 100 lines
az containerapp logs show \
  --name unitone-agw-dev-app \
  --resource-group unitone-agw-dev-rg \
  --tail 100
```

## Infrastructure Components

### Created Resources

| Resource | Purpose | Environment |
|----------|---------|-------------|
| **Container Registry** | Stores Docker images | `unitoneagw{env}acr` |
| **Key Vault** | Stores OAuth secrets | `unitone-agw-{env}-kv` |
| **Container App Env** | Hosts container apps | `unitone-agw-{env}-env` |
| **Container App** | Runs AgentGateway | `unitone-agw-{env}-app` |
| **Log Analytics** | Centralized logging | `unitone-agw-{env}-logs` |
| **App Insights** | Application monitoring | `unitone-agw-{env}-insights` |

### Scaling Configuration

- **Dev**: 1-3 replicas
- **Staging**: 1-5 replicas
- **Prod**: 2-10 replicas

Auto-scaling based on HTTP concurrent requests (100 per replica).

## Cost Estimate

| Environment | Monthly Cost |
|-------------|--------------|
| **Dev** | ~$15-30 |
| **Staging** | ~$30-60 |
| **Prod** | ~$100-300 |

Costs include: Container Apps, ACR, Key Vault, Log Analytics, Application Insights.

## Troubleshooting

### OAuth Secrets Not Found

```bash
# Verify secrets exist in Key Vault
az keyvault secret list --vault-name unitone-secrets-kv --query "[].name" -o table

# Check Container App can access Key Vault
az containerapp show \
  --name unitone-agw-dev-app \
  --resource-group unitone-agw-dev-rg \
  --query "identity.principalId"

# Verify Key Vault access policy
az keyvault show \
  --name unitone-agw-dev-kv \
  --query "properties.accessPolicies[?objectId=='<PRINCIPAL_ID>']"
```

### Container App Unhealthy

```bash
# Check replica status
az containerapp replica list \
  --name unitone-agw-dev-app \
  --resource-group unitone-agw-dev-rg \
  --output table

# View container logs
az containerapp logs show \
  --name unitone-agw-dev-app \
  --resource-group unitone-agw-dev-rg \
  --tail 50
```

### OAuth Flow Not Working

1. **Check redirect URIs** match your deployment URL
2. **Verify JWKS URL** is accessible: `curl https://token.actions.githubusercontent.com/.well-known/jwks`
3. **Test token validation** with your OAuth provider's token introspection endpoint
4. **Check CORS settings** in Bicep template

## Security Best Practices

1. **Rotate Secrets Regularly**:
   ```bash
   az keyvault secret set --vault-name unitone-secrets-kv --name agw-dev-github-client-secret --value "NEW_SECRET"
   # Restart Container App to pick up new secret
   az containerapp revision copy --name unitone-agw-dev-app --resource-group unitone-agw-dev-rg
   ```

2. **Use Managed Identities**: Already configured in Bicep template

3. **Enable HTTPS Only**: Configured in ingress settings

4. **Restrict CORS Origins**: Update `corsPolicy` in `main.bicep` for production

5. **Monitor Access**: Use Application Insights to track OAuth failures

## Comparison with MCP Scanner

| Feature | MCP Scanner | AgentGateway |
|---------|-------------|--------------|
| **Architecture** | Frontend + API + Worker | Single service |
| **Database** | PostgreSQL + Redis | Stateless (no DB) |
| **OAuth** | GitHub, Microsoft, Google | **Same** + Any OIDC provider |
| **OAuth Pattern** | Session-based (cookies) | Token-based (Bearer) |
| **Deployment** | 3 Container Apps | 1 Container App |
| **Cost** | ~$100-400/month | **~$15-300/month** |
| **Use Case** | Security scanning | **MCP Gateway/Proxy** |

## Next Steps

1. **Add Your Own MCP Servers**: Edit `deploy/configs/oauth-config.yaml`
2. **Configure Custom Domain**: Update `customDomain` in parameters
3. **Set up CI/CD**: Use GitHub Actions or Azure DevOps
4. **Add More OAuth Providers**: Auth0, Okta, Keycloak support included
5. **Enable Rate Limiting**: Add rate limit policies in config

## Support

For issues or questions:
- Check [AgentGateway Documentation](https://github.com/agentgateway/agentgateway)
- Review [MCP Authentication Spec](https://spec.modelcontextprotocol.io/specification/2025-11-05/authentication/)
- Open an issue on GitHub

---

**Created by**: UnitOne DevOps Team
**Based on**: MCP Scanner deployment pattern
**Last Updated**: December 2025
