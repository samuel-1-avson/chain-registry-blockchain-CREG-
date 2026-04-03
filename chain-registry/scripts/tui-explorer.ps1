# Chain Registry TUI Explorer Launcher (PowerShell version)
# Usage: .\scripts\tui-explorer.ps1 [dev|testnet|light]

param(
    [ValidateSet("dev", "single", "testnet", "light")]
    [string]$Mode = "dev"
)

# Determine which compose file to use
$ComposeFile = switch ($Mode) {
    "dev" { "docker-compose.yml" }
    "single" { "docker-compose.yml" }
    "testnet" { "docker-compose.testnet.yml" }
    "light" { "docker-compose.light.yml" }
    default { 
        Write-Host "Usage: .\tui-explorer.ps1 [dev|testnet|light]"
        Write-Host ""
        Write-Host "Modes:"
        Write-Host "  dev      - Single validator development setup (default)"
        Write-Host "  testnet  - 10-validator testnet"
        Write-Host "  light    - Resource-constrained light node"
        exit 1
    }
}

Write-Host "`u{1F680} Launching Chain Registry TUI Explorer..."
Write-Host "   Mode: $Mode"
Write-Host "   Compose file: $ComposeFile"
Write-Host ""

# Check if docker is available
try {
    $null = docker --version 2>$null
    if ($LASTEXITCODE -ne 0) { throw "Docker not found" }
} catch {
    Write-Host "`u{274C} Docker is not installed"
    exit 1
}

# Change to script directory
$scriptPath = Split-Path -Parent $MyInvocation.MyCommand.Path
if ($scriptPath) {
    Set-Location (Join-Path $scriptPath "..")
}

# Ensure the node is running
Write-Host "`u{1F50D} Checking if node is running..."
$nodeRunning = docker-compose -f $ComposeFile ps | Select-String "creg-node"

if (-not $nodeRunning) {
    Write-Host "`u{26A0}  Node is not running. Starting it first..."
    docker-compose -f $ComposeFile up -d
    
    # Wait for node to be healthy
    Write-Host "`u{23F3} Waiting for node to be ready..."
    Start-Sleep -Seconds 5
    
    $ready = $false
    for ($i = 1; $i -le 30; $i++) {
        try {
            $response = Invoke-RestMethod -Uri "http://localhost:8080/v1/health" -TimeoutSec 2 -ErrorAction SilentlyContinue
            if ($response) {
                Write-Host "`u{2705} Node is ready!"
                $ready = $true
                break
            }
        } catch {}
        Write-Host "   Still waiting... ($i/30)"
        Start-Sleep -Seconds 2
    }
    
    if (-not $ready) {
        Write-Warning "Node may not be fully ready, continuing anyway..."
    }
}

# Launch TUI explorer
Write-Host ""
Write-Host "`u{1F5A5}  Starting TUI Explorer..."
Write-Host "   Press '?' for help, 'q' to quit"
Write-Host ""

# Run with TTY allocation
docker-compose -f $ComposeFile run --rm tui-explorer

Write-Host ""
Write-Host "`u{1F44B} TUI Explorer closed"
