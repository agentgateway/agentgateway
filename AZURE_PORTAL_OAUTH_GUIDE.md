# Azure Portal: Easy Auth OAuth Configuration

## Step-by-Step Guide to Configure Web Login

This guide will walk you through configuring Microsoft and Google OAuth providers via Azure Portal.

---

## Prerequisites

Before starting, you need:
- Azure Portal access: https://portal.azure.com
- OAuth Client IDs and Secrets from your existing OAuth applications

---

## Part 1: Configure Microsoft OAuth Provider

### Step 1: Navigate to Container App Authentication

1. Open https://portal.azure.com in your browser
2. In the search bar at the top, type: `unitone-agentgateway`
3. Click on the Container App: **unitone-agentgateway**
4. In the left sidebar, scroll down to **Settings** section
5. Click on **Authentication**

### Step 2: Check Current Status

You should see:
- **Platform settings** section showing "Enabled"
- **Identity providers** section (currently empty or with default settings)

### Step 3: Add Microsoft Identity Provider

1. Click the **Add identity provider** button
2. From the dropdown, select **Microsoft**

### Step 4: Configure Microsoft Settings

You'll see a configuration form. Fill in these fields:

**App registration type:**
- Select: **Provide the details of an existing app registration**

**Application (client) ID:**
- This is your Microsoft OAuth Client ID from MCP Scanner
- You can find it in one of these places:
  - Azure AD > App registrations > Your MCP Scanner app
  - Or ask your admin who set up MCP Scanner

**Client secret:**
- **Option A**: Reference from Key Vault
  - Click **Add Key Vault reference**
  - Select Key Vault: `mcp-scanner-dev-kv`
  - Select Secret: `MICROSOFT-CLIENT-SECRET`

- **Option B**: Enter directly (if you have it)
  - Paste your Microsoft Client Secret value

**Issuer URL:**
```
https://login.microsoftonline.com/common/v2.0
```
(Use `common` for multi-tenant, or your specific tenant ID)

**Allowed token audiences:**
- Leave default or add:
```
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io
```

### Step 5: Save Microsoft Provider

1. Click **Add** button at the bottom
2. Wait for the configuration to be saved (you'll see a notification)

---

## Part 2: Configure Google OAuth Provider

### Step 1: Add Google Identity Provider

1. Still in the **Authentication** section
2. Click **Add identity provider** button again
3. From the dropdown, select **Google**

### Step 2: Configure Google Settings

**Client ID:**
- This is your Google OAuth Client ID from MCP Scanner
- You can find it in:
  - Google Cloud Console > Credentials
  - Or from whoever set up MCP Scanner OAuth

**Client secret:**
- **Option A**: Reference from Key Vault
  - Click **Add Key Vault reference**
  - Select Key Vault: `mcp-scanner-dev-kv`
  - Select Secret: `GOOGLE-CLIENT-SECRET`

- **Option B**: Enter directly (if you have it)
  - Paste your Google Client Secret value

**Allowed token audiences:** (Optional)
- Leave empty or default

### Step 3: Save Google Provider

1. Click **Add** button at the bottom
2. Wait for the configuration to be saved

---

## Part 3: Update Redirect URIs in Your OAuth Apps

Now you need to add redirect URIs to your actual OAuth applications.

### For Microsoft Azure AD App

1. Go to https://portal.azure.com
2. Navigate to: **Azure Active Directory** > **App registrations**
3. Find your MCP Scanner Microsoft app
4. Click on it to open
5. In the left menu, click **Authentication**
6. Under **Redirect URIs**, click **Add a platform** or **Add URI**
7. Add this URI:
```
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/aad/callback
```
8. Click **Save**

### For Google OAuth App

1. Go to https://console.cloud.google.com
2. Select your project
3. Go to **APIs & Services** > **Credentials**
4. Click on your OAuth 2.0 Client ID
5. Under **Authorized redirect URIs**, click **+ ADD URI**
6. Add this URI:
```
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/google/callback
```
7. Click **Save**

---

## Part 4: Configure Authentication Settings

### Step 1: Adjust Unauthenticated Access

1. Still in Container App **Authentication** section
2. Look for **Unauthenticated requests** or **Restrict access**
3. Select: **Allow unauthenticated access**
   - This is important! It allows token-based API access to continue working

### Step 2: Configure Token Store (Optional)

1. Under **Token store**, ensure it's **Enabled**
2. This allows Easy Auth to manage session tokens

---

## Part 5: Test Your Configuration

### Test Microsoft Login

1. Open your browser
2. Go to:
```
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/aad
```
3. You should be redirected to Microsoft login
4. Sign in with your Microsoft account
5. You should be redirected back to your app

### Test Google Login

1. Open your browser (or incognito window)
2. Go to:
```
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/google
```
3. You should be redirected to Google login
4. Sign in with your Google account
5. You should be redirected back to your app

### View Logged-in User Info

Once logged in, visit:
```
https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/me
```

You should see JSON with your authentication details.

---

## Troubleshooting

### "Redirect URI mismatch" Error

**Problem**: Getting an error about redirect URI not matching

**Solution**: Double-check you added the exact redirect URI to your OAuth app:
- Microsoft: `.../.auth/login/aad/callback`
- Google: `.../.auth/login/google/callback`

### Can't Find OAuth Credentials

**Problem**: Don't know your Client ID or Secret

**Solutions**:
1. Check with whoever set up MCP Scanner
2. Look in Azure AD > App registrations (for Microsoft)
3. Look in Google Cloud Console > Credentials (for Google)
4. Ask your team's admin

### "Not authorized" When Adding Key Vault Reference

**Problem**: Can't access secrets from Key Vault dropdown

**Solution**: Your Container App's managed identity needs access to the Key Vault:
```bash
# Get Container App's managed identity
az containerapp show --name unitone-agentgateway --resource-group mcp-gateway-dev-rg --query identity.principalId -o tsv

# Grant it access (run with admin privileges)
az keyvault set-policy --name mcp-scanner-dev-kv --object-id <PRINCIPAL_ID> --secret-permissions get list
```

### Token-Based API Access Stopped Working

**Problem**: API calls with Bearer tokens no longer work

**Solution**: Make sure **Unauthenticated requests** is set to **Allow unauthenticated access**, not **Require authentication**

---

## What You'll Have When Done

✅ **Web Login (Pattern 1)**: Users can sign in with Microsoft or Google via browser
✅ **Token-Based (Pattern 2)**: AI agents can still use Bearer tokens for API access
✅ **No Docker Rebuild**: All configured via Azure Portal settings

---

## Quick Reference

**Your Container App:**
- Name: `unitone-agentgateway`
- Resource Group: `mcp-gateway-dev-rg`
- URL: https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io

**Redirect URIs to Add:**
- Microsoft: `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/aad/callback`
- Google: `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/google/callback`

**Test URLs:**
- Microsoft login: `/.auth/login/aad`
- Google login: `/.auth/login/google`
- User info: `/.auth/me`
- Logout: `/.auth/logout`

---

## Alternative: If You Don't Have OAuth Credentials

If you don't have existing OAuth credentials, you can create new ones:

### Create New Microsoft App Registration

1. Azure Portal > Azure Active Directory > App registrations
2. Click **New registration**
3. Name: `unitone-agentgateway-oauth`
4. Redirect URI: `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/aad/callback`
5. Click **Register**
6. Copy the **Application (client) ID**
7. Go to **Certificates & secrets** > **New client secret**
8. Copy the secret value immediately (you won't see it again)

### Create New Google OAuth App

1. https://console.cloud.google.com
2. Create a project or select existing
3. APIs & Services > Credentials > Create Credentials > OAuth client ID
4. Application type: Web application
5. Name: `unitone-agentgateway-oauth`
6. Authorized redirect URI: `https://unitone-agentgateway.whitecliff-a0c9f0f7.eastus2.azurecontainerapps.io/.auth/login/google/callback`
7. Click **Create**
8. Copy Client ID and Client Secret

---

**Need Help?** Check EASY_AUTH_SETUP.md for additional troubleshooting or reach out to your Azure admin.
