# Quick Start: Enable Web Login on AgentGateway

## What You're Doing

Adding "Sign in with Microsoft/Google" buttons to your AgentGateway deployment.

## Current Status

✅ **Easy Auth Enabled** on Container App
✅ **Token-based OAuth Working** (Pattern 2)
⏳ **Web Login Pending** - Need to configure providers

---

## 5-Minute Setup via Azure Portal

### Step 1: Open Azure Portal

Go to: https://portal.azure.com

Search for: `unitone-agentgateway`

Click: **Authentication** (in left sidebar under Settings)

### Step 2: Add Microsoft Provider

1. Click **Add identity provider** → **Microsoft**
2. Fill in:
   - **App registration type**: Existing app registration
   - **Client ID**: Your Microsoft OAuth Client ID
   - **Client secret**: Your Microsoft Client Secret (or reference from Key Vault)
   - **Issuer URL**: `https://login.microsoftonline.com/common/v2.0`
3. Click **Add**

### Step 3: Add Google Provider

1. Click **Add identity provider** → **Google**
2. Fill in:
   - **Client ID**: Your Google OAuth Client ID
   - **Client secret**: Your Google Client Secret (or reference from Key Vault)
3. Click **Add**

### Step 4: Update OAuth App Redirect URIs

#### Microsoft (Azure AD):
1. Go to Azure AD → App registrations → Your app
2. Authentication → Add URI:
```
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/aad/callback
```

#### Google:
1. Go to https://console.cloud.google.com
2. Credentials → Your OAuth client → Add URI:
```
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/google/callback
```

### Step 5: Test It!

**Microsoft Login:**
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/aad

**Google Login:**
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/google

**View User Info:**
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/me

---

## Don't Have OAuth Credentials?

### Option 1: Use Existing MCP Scanner Credentials

Your MCP Scanner OAuth apps are stored in:
- Key Vault: `mcp-scanner-dev-kv`
- Secrets: `MICROSOFT-CLIENT-ID`, `MICROSOFT-CLIENT-SECRET`, `GOOGLE-CLIENT-ID`, `GOOGLE-CLIENT-SECRET`

Ask your admin or check Azure AD → App registrations

### Option 2: Create New OAuth Apps

See: `AZURE_PORTAL_OAUTH_GUIDE.md` → "Alternative: If You Don't Have OAuth Credentials"

---

## Detailed Guides Available

- **AZURE_PORTAL_OAUTH_GUIDE.md** - Complete step-by-step with screenshots descriptions
- **EASY_AUTH_SETUP.md** - Alternative CLI-based setup
- **configure-easy-auth.sh** - Automated script (requires credentials)

---

## What You'll Get

✅ Users can log in via browser with Microsoft or Google
✅ AI agents can still use Bearer tokens for API access
✅ Both authentication methods work simultaneously
✅ No Docker rebuild required

---

## Need Help?

Check the troubleshooting section in `AZURE_PORTAL_OAUTH_GUIDE.md` or ask your Azure admin for OAuth credentials.
