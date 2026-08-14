# Edit this value before uploading the script to Microsoft Intune.
$ExpectedCodexBaseUrl = "https://llm.example.com/v1"

$configured = $false
$path = Join-Path $env:USERPROFILE ".codex\managed_config.toml"

if (Test-Path -LiteralPath $path -PathType Leaf) {
    try {
        $configuration = [IO.File]::ReadAllText($path)
        $providerMatches = $configuration -match '(?m)^\s*model_provider\s*=\s*"agentgateway"\s*$'
        $sectionMatches = $configuration.Contains('[model_providers.agentgateway]')
        $urlMatches = $configuration.Contains("base_url = `"$ExpectedCodexBaseUrl`"")
        $wireApiMatches = $configuration -match '(?m)^\s*wire_api\s*=\s*"responses"\s*$'
        $configured = @(
            $providerMatches,
            $sectionMatches,
            $urlMatches,
            $wireApiMatches
        ) -notcontains $false
    } catch {
        Write-Error "Unable to read the Codex managed configuration."
        exit 1
    }
}

# Intune requires compressed JSON for Windows custom-compliance discovery.
$result = @{ CodexGatewayConfigured = $configured }
Write-Output ($result | ConvertTo-Json -Compress)
exit 0
