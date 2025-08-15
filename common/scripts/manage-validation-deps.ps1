# manage-validation-deps.ps1
# Usage: .\manage-validation-deps.ps1 start|stop

param(
    [Parameter(Mandatory=$true)]
    [ValidateSet("start", "stop")]
    [string]$Action
)

$KEYCLOAK_BASE_URL = "http://localhost:7080"
$KEYCLOAK_REALM_JWKS_ENDPOINT = "$KEYCLOAK_BASE_URL/realms/mcp/protocol/openid-connect/certs"

function Wait-ForHttpOk {
    param(
        [string]$Url,
        [int]$TimeoutSeconds = 120,
        [int]$SleepSeconds = 2
    )
    Write-Host "Waiting for $Url" -NoNewline
    $start = Get-Date
    while ($true) {
        try {
            $response = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 5
            Write-Host " - OK"
            return $true
        } catch {
            $now = Get-Date
            if (($now - $start).TotalSeconds -ge $TimeoutSeconds) {
                Write-Host " - TIMEOUT after ${TimeoutSeconds}s"
                return $false
            }
            Write-Host "." -NoNewline
            Start-Sleep -Seconds $SleepSeconds
        }
    }
}

switch ($Action) {
    "start" {
        Write-Host "Starting MCP authentication server..."
        Start-Process -FilePath "python" -ArgumentList "examples/mcp-authentication/auth_server.py" -WindowStyle Hidden

        Write-Host "Starting Keycloak..."
        Push-Location "examples/mcp-authentication/keycloak"
        docker compose up -d
        Pop-Location

        if (-not (Wait-ForHttpOk -Url $KEYCLOAK_REALM_JWKS_ENDPOINT -TimeoutSeconds 180 -SleepSeconds 3)) {
            Write-Error "Keycloak realm JWKS endpoint did not become available in time"
            exit 1
        }
    }
    "stop" {
        Get-Process | Where-Object { $_.Path -like "*auth_server.py*" } | ForEach-Object { $_.Kill() }
        Push-Location "examples/mcp-authentication/keycloak"
        docker compose down
        Pop-Location
    }
}
