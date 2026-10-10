# Set these values to match the Intune-managed client configuration before
# uploading the script.

# Exact Codex base_url, including /v1, and the matching TOML env_key name.
$ExpectedCodexBaseUrl = "https://llm.example.com/v1"
$ExpectedCodexEnvKey = "AGENTGATEWAY_API_KEY"

# Exact Claude Desktop inferenceGatewayBaseUrl. Include a route prefix, such
# as /claude, only when it is part of the managed Gateway URL.
$ExpectedClaudeGatewayUrl = "https://llm.example.com/claude"

# Use static for Gateway API key authentication or interactive for Entra ID.
$ExpectedClaudeCredentialKind = "static"

# Required only for interactive authentication. The flow is browser or broker.
$ExpectedClaudeOidcAuthFlow = ""
$ExpectedClaudeOidcIssuer = ""
$ExpectedClaudeOidcClientId = ""

# Enable only the clients and checks required by the assigned Intune group.
$VerifyCodex = $true
$VerifyClaudeDesktop = $true
$VerifyInstallation = $true
$VerifyNetwork = $true

$script:Failures = 0

function Write-Pass {
    param([Parameter(Mandatory = $true)][string]$Message)
    Write-Output "PASS: $Message"
}

function Write-Failure {
    param([Parameter(Mandatory = $true)][string]$Message)
    Write-Output "FAIL: $Message"
    $script:Failures++
}

function Test-CodexConfiguration {
    $knownPaths = @(
        (Join-Path $env:LOCALAPPDATA "Programs\Codex\Codex.exe"),
        (Join-Path $env:ProgramFiles "Codex\Codex.exe"),
        (Join-Path $env:USERPROFILE ".local\bin\codex.exe")
    )
    $installed = (Get-Command codex -ErrorAction SilentlyContinue) -or
        ($knownPaths | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf })

    if ($VerifyInstallation) {
        if ($installed) {
            Write-Pass "Codex is installed."
        } else {
            Write-Failure "Codex is not installed in a recognized location."
        }
    }

    $path = Join-Path $env:USERPROFILE ".codex\managed_config.toml"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Write-Failure "Codex managed configuration is missing."
        return
    }

    $configuration = [IO.File]::ReadAllText($path)
    $providerMatches = $configuration -match '(?m)^\s*model_provider\s*=\s*"agentgateway"\s*$'
    $sectionMatches = $configuration.Contains('[model_providers.agentgateway]')
    $urlMatches = $configuration.Contains("base_url = `"$ExpectedCodexBaseUrl`"")
    $wireApiMatches = $configuration -match '(?m)^\s*wire_api\s*=\s*"responses"\s*$'
    $envKeyPattern = '(?m)^\s*env_key\s*=\s*"' +
        [regex]::Escape($ExpectedCodexEnvKey) + '"\s*$'
    $envKeyMatches = $configuration -match $envKeyPattern

    $configurationMatches = @(
        $providerMatches,
        $sectionMatches,
        $urlMatches,
        $wireApiMatches,
        $envKeyMatches
    ) -notcontains $false

    if ($configurationMatches) {
        Write-Pass "Codex managed configuration uses the approved agentgateway URL and credential variable."
    } else {
        Write-Failure "Codex managed configuration does not match the approved provider, URL, wire API, and credential variable."
    }
}

function Test-ClaudeDesktopConfiguration {
    $knownPaths = @(
        (Join-Path $env:LOCALAPPDATA "Programs\Claude\Claude.exe"),
        (Join-Path $env:ProgramFiles "Claude\Claude.exe")
    )
    $installed = $knownPaths |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }

    if ($VerifyInstallation) {
        if ($installed) {
            Write-Pass "Claude Desktop is installed."
        } else {
            Write-Failure "Claude Desktop is not installed in a recognized location."
        }
    }

    $policyPath = "HKLM:\SOFTWARE\Policies\Claude"
    if (-not (Test-Path -LiteralPath $policyPath)) {
        Write-Failure "Claude Desktop machine policy is missing."
        return
    }

    $policy = Get-ItemProperty -LiteralPath $policyPath
    if ($policy.inferenceGatewayBaseUrl -ne $ExpectedClaudeGatewayUrl) {
        Write-Failure "Claude Desktop machine policy does not contain the approved agentgateway URL."
        return
    }

    if ($policy.inferenceCredentialKind -ne $ExpectedClaudeCredentialKind) {
        Write-Failure "Claude Desktop does not use the approved credential kind."
        return
    }

    if ($ExpectedClaudeCredentialKind -eq "static") {
        if ([string]::IsNullOrWhiteSpace([string]$policy.inferenceGatewayApiKey)) {
            Write-Failure "Claude Desktop static gateway credential is missing."
            return
        }
    } elseif ($ExpectedClaudeCredentialKind -eq "interactive") {
        if ([string]::IsNullOrWhiteSpace($ExpectedClaudeOidcAuthFlow) -or
            [string]::IsNullOrWhiteSpace($ExpectedClaudeOidcIssuer) -or
            [string]::IsNullOrWhiteSpace($ExpectedClaudeOidcClientId)) {
            Write-Failure "Claude Desktop interactive verification settings are incomplete."
            return
        }
        if ($policy.inferenceGatewayOidcAuthFlow -ne $ExpectedClaudeOidcAuthFlow) {
            Write-Failure "Claude Desktop does not use the approved OIDC sign-in flow."
            return
        }
        try {
            $oidc = $policy.inferenceGatewayOidc | ConvertFrom-Json
        } catch {
            Write-Failure "Claude Desktop OIDC settings are not valid JSON."
            return
        }
        if ($oidc.issuer -ne $ExpectedClaudeOidcIssuer -or
            $oidc.clientId -ne $ExpectedClaudeOidcClientId -or
            $oidc.bearerTokenType -ne "id_token") {
            Write-Failure "Claude Desktop OIDC settings do not match the approved issuer, client ID, and token type."
            return
        }
        if (-not [string]::IsNullOrWhiteSpace([string]$policy.inferenceGatewayApiKey)) {
            Write-Failure "Claude Desktop interactive configuration still contains a static gateway credential."
            return
        }
    } else {
        Write-Failure "Claude Desktop expected credential kind must be static or interactive."
        return
    }

    Write-Pass "Claude Desktop managed configuration uses the approved agentgateway URL and authentication settings."
}

function Test-AgentgatewayReachability {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string]$Url
    )

    try {
        $response = Invoke-WebRequest -Uri $Url -Method Head -UseBasicParsing `
            -TimeoutSec 15 -ErrorAction Stop
        Write-Pass "$Label received HTTP $([int]$response.StatusCode) from the approved agentgateway URL."
    } catch {
        if ($null -ne $_.Exception.Response) {
            $statusCode = [int]$_.Exception.Response.StatusCode
            Write-Pass "$Label received HTTP $statusCode from the approved agentgateway URL."
        } else {
            Write-Failure "$Label could not reach the approved agentgateway URL."
        }
    }
}

if ($VerifyCodex) {
    Test-CodexConfiguration
    if ($VerifyNetwork) {
        Test-AgentgatewayReachability -Label "Codex" -Url $ExpectedCodexBaseUrl
    }
}

if ($VerifyClaudeDesktop) {
    Test-ClaudeDesktopConfiguration
    if ($VerifyNetwork) {
        Test-AgentgatewayReachability -Label "Claude Desktop" -Url $ExpectedClaudeGatewayUrl
    }
}

if ($script:Failures -gt 0) {
    Write-Output "Verification failed with $($script:Failures) failed check(s)."
    exit 1
}

Write-Output "All enabled agentgateway client checks passed."
exit 0
