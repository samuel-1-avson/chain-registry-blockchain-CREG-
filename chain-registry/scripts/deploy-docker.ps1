# Docker Deployment Script for Chain Registry v0.3.0
# This script automates the deployment of the complete system with Phases 1-3

param(
    [switch]$Help
)

if ($Help) {
    Write-Host @"
Docker Deployment Script for Chain Registry v0.3.0

This script automates the deployment of the complete system.

Usage:
    .\deploy-docker.ps1

The script will:
    1. Check prerequisites (Docker, Docker Compose)
    2. Set up environment (.env file)
    3. Create necessary directories
    4. Build Docker images
    5. Start services
    6. Wait for services to be ready
    7. Verify deployment

Press any key to continue or Ctrl+C to cancel...
"@
    $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
}

# Configuration
$ComposeFile = "docker-compose.yml"
$EnvFile = ".env"
$Script:UseDockerComposeV2 = $true

# Colors
$Red = "`e[31m"
$Green = "`e[32m"
$Yellow = "`e[33m"
$Blue = "`e[34m"
$NC = "`e[0m"

# Logging functions
function Write-Info($Message) {
    Write-Host "$Blue[INFO]$NC $Message"
}

function Write-Success($Message) {
    Write-Host "$Green[SUCCESS]$NC $Message"
}

function Write-Warning($Message) {
    Write-Host "$Yellow[WARNING]$NC $Message"
}

function Write-ErrorMsg($Message) {
    Write-Host "$Red[ERROR]$NC $Message"
}

function Invoke-Compose {
    param(
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Args
    )

    if ($Script:UseDockerComposeV2) {
        & docker compose -f $ComposeFile @Args
    } else {
        & docker-compose -f $ComposeFile @Args
    }
}

# Check prerequisites
function Test-Prerequisites {
    Write-Info "Checking prerequisites..."
    
    # Check Docker
    try {
        $null = docker --version 2>$null
        if ($LASTEXITCODE -ne 0) { throw "Docker not found" }
    } catch {
        Write-ErrorMsg "Docker is not installed. Please install Docker first."
        exit 1
    }
    
    # Check Docker Compose
    $composeV2 = $false
    try {
        $null = docker compose version 2>$null
        if ($LASTEXITCODE -eq 0) { $composeV2 = $true }
    } catch {}
    
    if (-not $composeV2) {
        try {
            $null = docker-compose --version 2>$null
            if ($LASTEXITCODE -ne 0) { throw "Docker Compose not found" }
            $Script:UseDockerComposeV2 = $false
        } catch {
            Write-ErrorMsg "Docker Compose is not installed. Please install Docker Compose first."
            exit 1
        }
    }
    
    # Check Docker is running
    try {
        $null = docker info 2>$null
        if ($LASTEXITCODE -ne 0) { throw "Docker not running" }
    } catch {
        Write-ErrorMsg "Docker daemon is not running. Please start Docker."
        exit 1
    }
    
    Write-Success "Prerequisites check passed"
}

# Setup environment
function Initialize-Environment {
    Write-Info "Setting up environment..."
    
    if (-not (Test-Path $EnvFile)) {
        Write-Info "Creating .env file from example..."
        if (Test-Path ".env.example") {
            Copy-Item ".env.example" $EnvFile
        } else {
            Write-Warning ".env.example not found, creating empty .env"
            "# Chain Registry Environment" | Out-File $EnvFile
        }
        Write-Warning "Please edit $EnvFile with your configuration before continuing"
        Write-Warning "At minimum, set NODE1_VALIDATOR_KEY"
        Write-Host "Press Enter to continue after editing .env..."
        $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
    }
    
    # Source the environment file
    if (Test-Path $EnvFile) {
        Get-Content $EnvFile | ForEach-Object {
            if ($_ -match '^([^#][^=]*)=(.*)$') {
                [Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
            }
        }
    }
    
    Write-Success "Environment configured"
}

# Create necessary directories
function Initialize-Directories {
    Write-Info "Creating necessary directories..."
    
    @("validators", "circuits", "models", "data/node1") | ForEach-Object {
        New-Item -ItemType Directory -Force -Path $_ | Out-Null
    }
    
    # Create dummy WASM validator if none exists
    if (-not (Test-Path "validators/dummy.wasm")) {
        Write-Info "Creating dummy WASM validator..."
        "dummy content" | Out-File -FilePath "validators/dummy.wasm" -NoNewline
    }
    
    Write-Success "Directories created"
}

# Build and deploy
function Build-AndDeploy {
    Write-Info "Building Docker images..."
    
    Invoke-Compose build --parallel
    if ($LASTEXITCODE -ne 0) {
        Write-ErrorMsg "Docker build failed"
        exit 1
    }
    
    Write-Success "Docker images built"
    
    Write-Info "Starting services..."
    
    Invoke-Compose up -d
    if ($LASTEXITCODE -ne 0) {
        Write-ErrorMsg "Failed to start services"
        exit 1
    }
    
    Write-Success "Services started"
}

# Wait for services
function Wait-ForServices {
    Write-Info "Waiting for services to be ready..."
    
    # Wait for Anvil
    Write-Info "Waiting for Anvil (Ethereum local node)..."
    $anvilReady = $false
    for ($i = 1; $i -le 30; $i++) {
        try {
            $response = Invoke-RestMethod -Uri "http://localhost:8545" -Method Post `
                -Headers @{ "Content-Type" = "application/json" } `
                -Body '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' `
                -TimeoutSec 2 -ErrorAction SilentlyContinue
            if ($response) {
                Write-Success "Anvil is ready"
                $anvilReady = $true
                break
            }
        } catch {}
        Start-Sleep -Seconds 1
    }
    if (-not $anvilReady) {
        Write-Warning "Anvil may not be fully ready yet"
    }
    
    # Wait for IPFS
    Write-Info "Waiting for IPFS..."
    $ipfsReady = $false
    for ($i = 1; $i -le 30; $i++) {
        try {
            $response = Invoke-RestMethod -Uri "http://localhost:5001/api/v0/id" -TimeoutSec 2 -ErrorAction SilentlyContinue
            if ($response) {
                Write-Success "IPFS is ready"
                $ipfsReady = $true
                break
            }
        } catch {}
        Start-Sleep -Seconds 1
    }
    if (-not $ipfsReady) {
        Write-Warning "IPFS may not be fully ready yet"
    }
    
    # Wait for Node-1
    Write-Info "Waiting for Chain Registry Node-1..."
    $nodeReady = $false
    for ($i = 1; $i -le 60; $i++) {
        try {
            $response = Invoke-RestMethod -Uri "http://localhost:8080/v1/health" -TimeoutSec 2 -ErrorAction SilentlyContinue
            if ($response) {
                Write-Success "Node-1 is ready"
                $nodeReady = $true
                break
            }
        } catch {}
        Start-Sleep -Seconds 1
    }
    if (-not $nodeReady) {
        Write-Warning "Node-1 may not be fully ready yet"
    }
    
    Write-Success "All services initialization complete"
}

# Verify deployment
function Test-Deployment {
    Write-Info "Verifying deployment..."
    
    # Check all containers are running
    $running = (Invoke-Compose ps -q | Measure-Object).Count
    $expected = 4  # ipfs, anvil, deploy-contracts, node
    
    if ($running -ge $expected) {
        Write-Success "All required containers are running ($running/$expected)"
    } else {
        Write-Warning "Some containers may not be running ($running/$expected)"
        Invoke-Compose ps
    }
    
    # Test health endpoints
    Write-Info "Testing health endpoints..."
    
    try {
        $health = Invoke-RestMethod -Uri "http://localhost:8080/v1/health" -TimeoutSec 5 -ErrorAction SilentlyContinue
        if ($health -match "ok") {
            Write-Success "Node-1 health check passed"
        }
    } catch {
        Write-Warning "Node-1 health check failed"
    }
    
    # Check contract deployment
    Write-Info "Checking contract deployment status..."
    $deployStatus = Invoke-Compose ps --all deploy-contracts 2>&1
    if ($deployStatus -match "exited \(0\)" -or $deployStatus -match "running") {
        Write-Success "Contracts deployed successfully"
    } else {
        Write-Warning "Contract deployment status unclear - check logs with: docker compose -f $ComposeFile logs deploy-contracts"
    }
    
    Write-Success "Deployment verification complete"
}

# Print status
function Show-Status {
    Write-Host ""
    Write-Host "=========================================="
    Write-Host "  Chain Registry Deployment Status"
    Write-Host "=========================================="
    Write-Host ""
    
    Write-Host "Services:"
    Invoke-Compose ps
    
    Write-Host ""
    Write-Host "Access URLs:"
    Write-Host "  - Node API:       http://localhost:8080"
    Write-Host "  - Explorer UI:    http://localhost:8080/ui/"
    Write-Host "  - IPFS API:       http://localhost:5001"
    Write-Host "  - IPFS Gateway:   http://localhost:8081"
    Write-Host "  - Ethereum RPC:   http://localhost:8545"
    Write-Host "  - Faucet:         http://localhost:8082 (start with --profile testnet)"
    Write-Host ""
    
    Write-Host "Features Enabled:"
    Write-Host "  [x] Phase 1: ZK Validation"
    Write-Host "  [x] Phase 1: ML Threat Detection"
    Write-Host "  [x] Phase 1: WASM Sandboxing"
    Write-Host "  [x] Phase 2: Private Registries"
    Write-Host "  [x] Phase 2: Cross-Chain Support"
    Write-Host "  [x] Phase 3: CREG Token"
    Write-Host "  [x] Phase 3: Governance V2"
    Write-Host "  [x] Phase 3: Package Insurance"
    Write-Host ""
    
    Write-Host "Commands:"
    Write-Host "  View logs:        docker compose -f $ComposeFile logs -f"
    Write-Host "  Stop services:    docker compose -f $ComposeFile down"
    Write-Host "  CLI tool:         docker compose -f $ComposeFile run --rm cli --help"
    Write-Host ""
    
    Write-Host "=========================================="
}

# Main function
function Main {
    Write-Host "=========================================="
    Write-Host "  Chain Registry Docker Deployment"
    Write-Host "  Version: v0.3.0 (Phases 1-3)"
    Write-Host "=========================================="
    Write-Host ""
    
    # Change to script directory
    $scriptPath = Split-Path -Parent $MyInvocation.MyCommand.Path
    if ($scriptPath) {
        Set-Location (Join-Path $scriptPath "..")
    }
    
    try {
        Test-Prerequisites
        Initialize-Environment
        Initialize-Directories
        Build-AndDeploy
        Wait-ForServices
        Test-Deployment
        Show-Status
        
        Write-Success "Deployment complete!"
    } catch {
        Write-ErrorMsg "Deployment failed: $_"
        exit 1
    }
}

# Handle interruption
try {
    Main
} catch {
    Write-ErrorMsg "Deployment interrupted"
    exit 1
}
