# Edit these values before uploading the script to Microsoft Intune.
$ExpectedCodexBaseUrl = "https://llm.example.com/v1"
$ExpectedClaudeGatewayUrl = "https://llm.example.com/claude"
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

    if ($providerMatches -and $sectionMatches -and $urlMatches -and $wireApiMatches) {
        Write-Pass "Codex managed configuration uses the approved agentgateway URL."
    } else {
        Write-Failure "Codex managed configuration does not match the approved provider, URL, and wire API."
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
    $urlMatches = $policy.PSObject.Properties |
        Where-Object { $_.Name -notlike 'PS*' } |
        Where-Object { [string]$_.Value -like "*$ExpectedClaudeGatewayUrl*" }

    if ($urlMatches) {
        Write-Pass "Claude Desktop managed configuration uses the approved agentgateway URL."
    } else {
        Write-Failure "Claude Desktop machine policy does not contain the approved agentgateway URL."
    }
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
