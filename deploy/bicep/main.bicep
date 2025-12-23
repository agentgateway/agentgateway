// =============================================================================
// UnitOne AgentGateway - Azure Bicep Deployment
// Based on MCP Scanner deployment pattern
// =============================================================================

targetScope = 'resourceGroup'

// Parameters
@description('Environment name (dev, staging, prod)')
@allowed(['dev', 'staging', 'prod'])
param environment string = 'dev'

@description('Location for all resources')
param location string = resourceGroup().location

@description('Base name for all resources')
param baseName string = 'unitone-agw'

@description('Docker image tag to deploy')
param imageTag string = 'latest'

@description('GitHub OAuth Client ID')
@secure()
param githubClientId string = ''

@description('GitHub OAuth Client Secret')
@secure()
param githubClientSecret string = ''

@description('Microsoft OAuth Client ID')
@secure()
param microsoftClientId string = ''

@description('Microsoft OAuth Client Secret')
@secure()
param microsoftClientSecret string = ''

@description('Google OAuth Client ID')
@secure()
param googleClientId string = ''

@description('Google OAuth Client Secret')
@secure()
param googleClientSecret string = ''

@description('Custom domain name (optional)')
param customDomain string = ''

@description('ACR admin password')
@secure()
param acrPassword string

// Variables
var resourcePrefix = '${baseName}-${environment}'
var acrName = replace('${baseName}${environment}acr', '-', '')
var keyVaultName = '${resourcePrefix}-kv'
var logAnalyticsName = '${resourcePrefix}-logs'
var appInsightsName = '${resourcePrefix}-insights'
var containerAppEnvName = '${resourcePrefix}-env'
var containerAppName = '${resourcePrefix}-app'

// Tags
var tags = {
  Environment: environment
  Project: 'UnitOne AgentGateway'
  ManagedBy: 'Bicep'
}

// =============================================================================
// 1. Container Registry (ACR)
// =============================================================================
resource acr 'Microsoft.ContainerRegistry/registries@2023-01-01-preview' = {
  name: acrName
  location: location
  tags: tags
  sku: {
    name: 'Basic'
  }
  properties: {
    adminUserEnabled: true
    publicNetworkAccess: 'Enabled'
  }
}

// =============================================================================
// 2. Log Analytics Workspace
// =============================================================================
resource logAnalytics 'Microsoft.OperationalInsights/workspaces@2022-10-01' = {
  name: logAnalyticsName
  location: location
  tags: tags
  properties: {
    sku: {
      name: 'PerGB2018'
    }
    retentionInDays: 30
  }
}

// =============================================================================
// 3. Application Insights
// =============================================================================
resource appInsights 'Microsoft.Insights/components@2020-02-02' = {
  name: appInsightsName
  location: location
  tags: tags
  kind: 'web'
  properties: {
    Application_Type: 'web'
    WorkspaceResourceId: logAnalytics.id
  }
}

// =============================================================================
// 4. Key Vault for OAuth Secrets
// =============================================================================
resource keyVault 'Microsoft.KeyVault/vaults@2023-02-01' = {
  name: keyVaultName
  location: location
  tags: tags
  properties: {
    sku: {
      family: 'A'
      name: 'standard'
    }
    tenantId: subscription().tenantId
    enableRbacAuthorization: false
    accessPolicies: []
    enabledForDeployment: true
    enabledForTemplateDeployment: true
  }
}

// Store OAuth secrets in Key Vault
resource githubClientIdSecret 'Microsoft.KeyVault/vaults/secrets@2023-02-01' = if (!empty(githubClientId)) {
  parent: keyVault
  name: 'github-client-id'
  properties: {
    value: githubClientId
  }
}

resource githubClientSecretSecret 'Microsoft.KeyVault/vaults/secrets@2023-02-01' = if (!empty(githubClientSecret)) {
  parent: keyVault
  name: 'github-client-secret'
  properties: {
    value: githubClientSecret
  }
}

resource microsoftClientIdSecret 'Microsoft.KeyVault/vaults/secrets@2023-02-01' = if (!empty(microsoftClientId)) {
  parent: keyVault
  name: 'microsoft-client-id'
  properties: {
    value: microsoftClientId
  }
}

resource microsoftClientSecretSecret 'Microsoft.KeyVault/vaults/secrets@2023-02-01' = if (!empty(microsoftClientSecret)) {
  parent: keyVault
  name: 'microsoft-client-secret'
  properties: {
    value: microsoftClientSecret
  }
}

resource googleClientIdSecret 'Microsoft.KeyVault/vaults/secrets@2023-02-01' = if (!empty(googleClientId)) {
  parent: keyVault
  name: 'google-client-id'
  properties: {
    value: googleClientId
  }
}

resource googleClientSecretSecret 'Microsoft.KeyVault/vaults/secrets@2023-02-01' = if (!empty(googleClientSecret)) {
  parent: keyVault
  name: 'google-client-secret'
  properties: {
    value: googleClientSecret
  }
}

// =============================================================================
// 5. Container Apps Environment
// =============================================================================
resource containerAppEnv 'Microsoft.App/managedEnvironments@2023-05-01' = {
  name: containerAppEnvName
  location: location
  tags: tags
  properties: {
    appLogsConfiguration: {
      destination: 'log-analytics'
      logAnalyticsConfiguration: {
        customerId: logAnalytics.properties.customerId
        sharedKey: logAnalytics.listKeys().primarySharedKey
      }
    }
  }
}

// =============================================================================
// 6. Container App with Managed Identity
// =============================================================================
resource containerApp 'Microsoft.App/containerApps@2023-05-01' = {
  name: containerAppName
  location: location
  tags: tags
  identity: {
    type: 'SystemAssigned'
  }
  properties: {
    managedEnvironmentId: containerAppEnv.id
    configuration: {
      activeRevisionsMode: 'Single'
      ingress: {
        external: true
        targetPort: 8080
        transport: 'auto'
        allowInsecure: false
        traffic: [
          {
            latestRevision: true
            weight: 100
          }
        ]
        customDomains: !empty(customDomain) ? [
          {
            name: customDomain
            bindingType: 'SniEnabled'
          }
        ] : []
        corsPolicy: {
          allowedOrigins: [
            '*'
          ]
          allowedHeaders: [
            'mcp-protocol-version'
            'content-type'
            'authorization'
          ]
          allowedMethods: [
            'GET'
            'POST'
            'PUT'
            'DELETE'
            'OPTIONS'
          ]
          allowCredentials: true
        }
      }
      registries: [
        {
          server: acr.properties.loginServer
          username: acr.name
          passwordSecretRef: 'acr-password'
        }
      ]
      secrets: [
        {
          name: 'acr-password'
          value: acrPassword
        }
        {
          name: 'github-client-id'
          keyVaultUrl: !empty(githubClientId) ? githubClientIdSecret.properties.secretUri : ''
          identity: 'system'
        }
        {
          name: 'github-client-secret'
          keyVaultUrl: !empty(githubClientSecret) ? githubClientSecretSecret.properties.secretUri : ''
          identity: 'system'
        }
        {
          name: 'microsoft-client-id'
          keyVaultUrl: !empty(microsoftClientId) ? microsoftClientIdSecret.properties.secretUri : ''
          identity: 'system'
        }
        {
          name: 'microsoft-client-secret'
          keyVaultUrl: !empty(microsoftClientSecret) ? microsoftClientSecretSecret.properties.secretUri : ''
          identity: 'system'
        }
        {
          name: 'google-client-id'
          keyVaultUrl: !empty(googleClientId) ? googleClientIdSecret.properties.secretUri : ''
          identity: 'system'
        }
        {
          name: 'google-client-secret'
          keyVaultUrl: !empty(googleClientSecret) ? googleClientSecretSecret.properties.secretUri : ''
          identity: 'system'
        }
      ]
    }
    template: {
      containers: [
        {
          name: 'agentgateway'
          image: '${acr.properties.loginServer}/unitone-agentgateway:${imageTag}'
          resources: {
            cpu: json('1.0')
            memory: '2Gi'
          }
          env: [
            {
              name: 'APPLICATIONINSIGHTS_CONNECTION_STRING'
              value: appInsights.properties.ConnectionString
            }
            {
              name: 'GITHUB_CLIENT_ID'
              secretRef: 'github-client-id'
            }
            {
              name: 'GITHUB_CLIENT_SECRET'
              secretRef: 'github-client-secret'
            }
            {
              name: 'MICROSOFT_CLIENT_ID'
              secretRef: 'microsoft-client-id'
            }
            {
              name: 'MICROSOFT_CLIENT_SECRET'
              secretRef: 'microsoft-client-secret'
            }
            {
              name: 'GOOGLE_CLIENT_ID'
              secretRef: 'google-client-id'
            }
            {
              name: 'GOOGLE_CLIENT_SECRET'
              secretRef: 'google-client-secret'
            }
          ]
        }
      ]
      scale: {
        minReplicas: environment == 'prod' ? 2 : 1
        maxReplicas: environment == 'prod' ? 10 : 3
        rules: [
          {
            name: 'http-scaling'
            http: {
              metadata: {
                concurrentRequests: '100'
              }
            }
          }
        ]
      }
    }
  }
}

// =============================================================================
// 7. Key Vault Access Policy for Container App Managed Identity
// =============================================================================
resource keyVaultAccessPolicy 'Microsoft.KeyVault/vaults/accessPolicies@2023-02-01' = {
  parent: keyVault
  name: 'add'
  properties: {
    accessPolicies: [
      {
        tenantId: subscription().tenantId
        objectId: containerApp.identity.principalId
        permissions: {
          secrets: [
            'get'
            'list'
          ]
        }
      }
    ]
  }
}

// =============================================================================
// Outputs
// =============================================================================
output containerAppFQDN string = containerApp.properties.configuration.ingress.fqdn
output containerAppUrl string = 'https://${containerApp.properties.configuration.ingress.fqdn}'
output acrLoginServer string = acr.properties.loginServer
output keyVaultName string = keyVault.name
output containerAppName string = containerApp.name
output resourceGroupName string = resourceGroup().name
output environment string = environment
