# Set these values to match the Intune-managed Claude Desktop profile before
# uploading the script.

# Exact inferenceGatewayBaseUrl. Include a route prefix, such as /claude, only
# when it is part of the managed Gateway URL.
$ExpectedClaudeGatewayUrl = "https://llm.example.com/claude"

# Use static for Gateway API key authentication or interactive for Entra ID.
$ExpectedClaudeCredentialKind = "static"

# Required only for interactive authentication. The flow is browser or broker.
$ExpectedClaudeOidcAuthFlow = ""
$ExpectedClaudeOidcIssuer = ""
$ExpectedClaudeOidcClientId = ""

$configured = $false
$policyPath = "HKLM:\SOFTWARE\Policies\Claude"

if (Test-Path -LiteralPath $policyPath) {
    try {
        $policy = Get-ItemProperty -LiteralPath $policyPath
        $providerMatches = $policy.inferenceProvider -eq "gateway"
        $urlMatches = $policy.inferenceGatewayBaseUrl -eq $ExpectedClaudeGatewayUrl
        $credentialMatches = $policy.inferenceCredentialKind -eq $ExpectedClaudeCredentialKind
        $authenticationMatches = $false

        if ($ExpectedClaudeCredentialKind -eq "static") {
            $authenticationMatches = -not [string]::IsNullOrWhiteSpace(
                [string]$policy.inferenceGatewayApiKey
            )
        } elseif ($ExpectedClaudeCredentialKind -eq "interactive") {
            try {
                $oidc = $policy.inferenceGatewayOidc | ConvertFrom-Json
                $authenticationMatches =
                    -not [string]::IsNullOrWhiteSpace($ExpectedClaudeOidcAuthFlow) -and
                    -not [string]::IsNullOrWhiteSpace($ExpectedClaudeOidcIssuer) -and
                    -not [string]::IsNullOrWhiteSpace($ExpectedClaudeOidcClientId) -and
                    $policy.inferenceGatewayOidcAuthFlow -eq $ExpectedClaudeOidcAuthFlow -and
                    $oidc.issuer -eq $ExpectedClaudeOidcIssuer -and
                    $oidc.clientId -eq $ExpectedClaudeOidcClientId -and
                    $oidc.bearerTokenType -eq "id_token" -and
                    [string]::IsNullOrWhiteSpace([string]$policy.inferenceGatewayApiKey)
            } catch {
                $authenticationMatches = $false
            }
        } else {
            $authenticationMatches = $false
        }

        $configured = $providerMatches -and $urlMatches -and
            $credentialMatches -and $authenticationMatches
    } catch {
        Write-Error "Unable to read the Claude Desktop machine policy."
        exit 1
    }
}

# Intune requires compressed JSON for Windows custom-compliance discovery.
$result = @{ ClaudeDesktopGatewayConfigured = $configured }
Write-Output ($result | ConvertTo-Json -Compress)
exit 0
