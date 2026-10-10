# Set these values to match the Intune-managed Codex TOML before uploading the
# script. Use the exact base_url, including /v1, and matching env_key name.
$ExpectedCodexBaseUrl = "https://llm.example.com/v1"
$ExpectedCodexEnvKey = "AGENTGATEWAY_API_KEY"

$configured = $false
$path = Join-Path $env:USERPROFILE ".codex\managed_config.toml"

if (Test-Path -LiteralPath $path -PathType Leaf) {
    try {
        $configuration = [IO.File]::ReadAllText($path)
        $providerMatches = $configuration -match '(?m)^\s*model_provider\s*=\s*"agentgateway"\s*$'
        $sectionMatches = $configuration.Contains('[model_providers.agentgateway]')
        $urlMatches = $configuration.Contains("base_url = `"$ExpectedCodexBaseUrl`"")
        $wireApiMatches = $configuration -match '(?m)^\s*wire_api\s*=\s*"responses"\s*$'
        $envKeyPattern = '(?m)^\s*env_key\s*=\s*"' +
            [regex]::Escape($ExpectedCodexEnvKey) + '"\s*$'
        $envKeyMatches = $configuration -match $envKeyPattern
        $configured = @(
            $providerMatches,
            $sectionMatches,
            $urlMatches,
            $wireApiMatches,
            $envKeyMatches
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
