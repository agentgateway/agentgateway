# Edit this value before uploading the script to Microsoft Intune.
$ExpectedClaudeGatewayUrl = "https://llm.example.com/claude"

$configured = $false
$policyPath = "HKLM:\SOFTWARE\Policies\Claude"

if (Test-Path -LiteralPath $policyPath) {
    try {
        $policy = Get-ItemProperty -LiteralPath $policyPath
        $providerMatches = $policy.inferenceProvider -eq "gateway"
        $urlMatches = $policy.inferenceGatewayBaseUrl -eq $ExpectedClaudeGatewayUrl
        $configured = $providerMatches -and $urlMatches
    } catch {
        Write-Error "Unable to read the Claude Desktop machine policy."
        exit 1
    }
}

# Intune requires compressed JSON for Windows custom-compliance discovery.
$result = @{ ClaudeDesktopGatewayConfigured = $configured }
Write-Output ($result | ConvertTo-Json -Compress)
exit 0
