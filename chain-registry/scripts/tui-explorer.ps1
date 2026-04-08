# Chain Registry TUI Explorer Launcher (PowerShell version)
# Usage: .\scripts\tui-explorer.ps1 [dev|testnet|light]

param(
    [ValidateSet("dev", "single", "testnet", "light")]
    [string]$Mode = "dev"
)

# Determine which compose file and node container to use
$ComposeFile = $null
$NodeService = $null
$NodeContainer = $null
$ComposeExtraArgs = @()

switch ($Mode) {
    "dev" {
        $ComposeFile = "docker-compose.yml"
        $NodeService = "node"
        $NodeContainer = "creg-node"
    }
    "single" {
        $ComposeFile = "docker-compose.yml"
        $NodeService = "node"
        $NodeContainer = "creg-node"
    }
    "testnet" {
        $ComposeFile = "docker-compose.testnet.yml"
        $NodeService = "node-1"
        $NodeContainer = "creg-testnet-node-1"
        $ComposeExtraArgs = @("--env-file", ".env.testnet")
    }
    "light" {
        $ComposeFile = "docker-compose.light.yml"
        $NodeService = "node-light"
        $NodeContainer = "creg-node-light"
    }
    default { 
        Write-Host "Usage: .\tui-explorer.ps1 [dev|testnet|light]"
        Write-Host ""
        Write-Host "Modes:"
        Write-Host "  dev      - Single validator development setup (default)"
        Write-Host "  testnet  - Single-validator testnet bootstrap host"
        Write-Host "  light    - Resource-constrained light node"
        exit 1
    }
}

function Invoke-Compose {
    param([string[]]$Args)
    & docker compose @ComposeExtraArgs -f $ComposeFile @Args
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
$nodeRunning = docker ps --filter "name=^/${NodeContainer}$" --format "{{.Names}}"

if (-not $nodeRunning) {
    Write-Host "`u{26A0}  Node is not running. Starting it first..."
    if ($Mode -eq "testnet") {
        Invoke-Compose @("up", "-d", "ipfs", "anvil", "postgres")
        Invoke-Compose @("up", "-d", "--no-deps", "node-1", "faucet", "web-explorer")
    } else {
        Invoke-Compose @("up", "-d", $NodeService)
    }
    
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

try {
    docker exec -it $NodeContainer /app/creg console --node-url http://127.0.0.1:8080
} catch {
    Write-Warning "Direct console attach failed, falling back to one-shot TUI container..."
    Invoke-Compose @("run", "--rm", "--no-deps", "tui-explorer")
}

Write-Host ""
Write-Host "`u{1F44B} TUI Explorer closed"
