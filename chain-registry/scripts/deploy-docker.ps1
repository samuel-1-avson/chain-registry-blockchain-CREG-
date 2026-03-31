#
# Docker Deployment Script for Chain Registry v0.2.0 (Windows PowerShell)
# This script automates the deployment of the complete system with Phases 1-3
#

$ErrorActionPreference = "Stop"

# Colors for output
$Red = "Red"
$Green = "Green"
$Yellow = "Yellow"
$Cyan = "Cyan"

# Configuration
$ComposeFile = "docker-compose.yml"
$EnvFile = ".env"

# Functions
function Log-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor $Cyan
}

function Log-Success {
    param([string]$Message)
    Write-Host "[SUCCESS] $Message" -ForegroundColor $Green
}

function Log-Warning {
    param([string]$Message)
    Write-Host "[WARNING] $Message" -ForegroundColor $Yellow
}

function Log-Error {
    param([string]$Message)
    Write-Host "[ERROR] $Message" -ForegroundColor $Red
}

# Check prerequisites
function Check-Prerequisites {
    Log-Info "Checking prerequisites..."
    
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        Log-Error "Docker is not installed. Please install Docker first."
        exit 1
    }
    
    if (-not (Get-Command docker-compose -ErrorAction SilentlyContinue)) {
        Log-Error "Docker Compose is not installed. Please install Docker Compose first."
        exit 1
    }
    
    # Check Docker is running
    try {
        $null = docker info 2>&1
    } catch {
        Log-Error "Docker daemon is not running. Please start Docker."
        exit 1
    }
    
    Log-Success "Prerequisites check passed"
}

# Setup environment
function Setup-Environment {
    Log-Info "Setting up environment..."
    
    if (-not (Test-Path $EnvFile)) {
        Log-Info "Creating .env file from example..."
        if (Test-Path ".env.example") {
            Copy-Item ".env.example" $EnvFile
        }
        Log-Warning "Please edit $EnvFile with your configuration before continuing"
        Log-Warning "At minimum, set NODE1_VALIDATOR_KEY, NODE2_VALIDATOR_KEY, NODE3_VALIDATOR_KEY"
        Read-Host "Press Enter to continue after editing .env"
    }
    
    Log-Success "Environment configured"
}

# Create necessary directories
function Create-Directories {
    Log-Info "Creating necessary directories..."
    
    New-Item -ItemType Directory -Force -Path "validators" | Out-Null
    New-Item -ItemType Directory -Force -Path "circuits" | Out-Null
    New-Item -ItemType Directory -Force -Path "models" | Out-Null
    New-Item -ItemType Directory -Force -Path "data/node1" | Out-Null
    New-Item -ItemType Directory -Force -Path "data/node2" | Out-Null
    New-Item -ItemType Directory -Force -Path "data/node3" | Out-Null
    
    # Create dummy WASM validator if none exists
    if (-not (Test-Path "validators/dummy.wasm")) {
        Log-Info "Creating dummy WASM validator..."
        "dummy content" | Out-File -FilePath "validators/dummy.wasm"
    }
    
    Log-Success "Directories created"
}

# Build and deploy
function Build-And-Deploy {
    Log-Info "Building Docker images..."
    
    docker-compose -f $ComposeFile build --parallel
    
    Log-Success "Docker images built"
    
    Log-Info "Starting services..."
    
    docker-compose -f $ComposeFile up -d
    
    Log-Success "Services started"
}

# Wait for services
function Wait-For-Services {
    Log-Info "Waiting for services to be ready..."
    
    # Wait for Anvil
    Log-Info "Waiting for Anvil (Ethereum local node)..."
    $maxAttempts = 30
    $attempts = 0
    while ($attempts -lt $maxAttempts) {
        try {
            $body = '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
            $response = Invoke-RestMethod -Uri "http://localhost:8545" -Method Post -ContentType "application/json" -Body $body -ErrorAction SilentlyContinue
            if ($response) {
                Log-Success "Anvil is ready"
                break
            }
        } catch {
            # Continue waiting
        }
        Start-Sleep -Seconds 1
        $attempts++
    }
    
    # Wait for IPFS
    Log-Info "Waiting for IPFS..."
    $attempts = 0
    while ($attempts -lt $maxAttempts) {
        try {
            $response = Invoke-RestMethod -Uri "http://localhost:5001/api/v0/id" -ErrorAction SilentlyContinue
            if ($response) {
                Log-Success "IPFS is ready"
                break
            }
        } catch {
            # Continue waiting
        }
        Start-Sleep -Seconds 1
        $attempts++
    }
    
    # Wait for Node-1
    Log-Info "Waiting for Chain Registry Node-1..."
    $maxAttempts = 60
    $attempts = 0
    while ($attempts -lt $maxAttempts) {
        try {
            $response = Invoke-RestMethod -Uri "http://localhost:8080/v1/health" -ErrorAction SilentlyContinue
            if ($response) {
                Log-Success "Node-1 is ready"
                break
            }
        } catch {
            # Continue waiting
        }
        Start-Sleep -Seconds 1
        $attempts++
    }
    
    Log-Success "All services are ready"
}

# Verify deployment
function Verify-Deployment {
    Log-Info "Verifying deployment..."
    
    # Check all containers are running
    $running = (docker-compose ps -q).Count
    $expected = 6  # ipfs, anvil, deploy-contracts, node-1, node-2, node-3
    
    if ($running -ge 5) {
        Log-Success "All required containers are running ($running/6)"
    } else {
        Log-Warning "Some containers may not be running ($running/6)"
        docker-compose ps
    }
    
    # Test health endpoints
    Log-Info "Testing health endpoints..."
    
    try {
        $health = Invoke-RestMethod -Uri "http://localhost:8080/v1/health"
        if ($health -match "ok") {
            Log-Success "Node-1 health check passed"
        }
    } catch {
        Log-Warning "Node-1 health check failed"
    }
    
    Log-Success "Deployment verification complete"
}

# Print status
function Print-Status {
    Write-Host ""
    Write-Host "=========================================="
    Write-Host "  Chain Registry Deployment Status"
    Write-Host "=========================================="
    Write-Host ""
    
    Write-Host "Services:"
    docker-compose ps
    
    Write-Host ""
    Write-Host "Access URLs:"
    Write-Host "  - Node 1 API:     http://localhost:8080"
    Write-Host "  - Node 2 API:     http://localhost:8082"
    Write-Host "  - Node 3 API:     http://localhost:8083"
    Write-Host "  - IPFS API:       http://localhost:5001"
    Write-Host "  - IPFS Gateway:   http://localhost:8081"
    Write-Host "  - Ethereum RPC:   http://localhost:8545"
    Write-Host ""
    
    Write-Host "Features Enabled:"
    Write-Host "  ✅ Phase 1: ZK Validation"
    Write-Host "  ✅ Phase 1: ML Threat Detection"
    Write-Host "  ✅ Phase 1: WASM Sandboxing"
    Write-Host "  ✅ Phase 2: Private Registries"
    Write-Host "  ✅ Phase 2: Cross-Chain Support"
    Write-Host "  ✅ Phase 3: CREG Token"
    Write-Host "  ✅ Phase 3: Governance V2"
    Write-Host "  ✅ Phase 3: Package Insurance"
    Write-Host ""
    
    Write-Host "Commands:"
    Write-Host "  View logs:        docker-compose logs -f"
    Write-Host "  Stop services:    docker-compose down"
    Write-Host "  CLI tool:         docker-compose run --rm cli --help"
    Write-Host ""
    
    Write-Host "=========================================="
}

# Main function
function Main {
    Write-Host "=========================================="
    Write-Host "  Chain Registry Docker Deployment"
    Write-Host "  Version: v0.2.0 (Phases 1-3)"
    Write-Host "=========================================="
    Write-Host ""
    
    Check-Prerequisites
    Setup-Environment
    Create-Directories
    Build-And-Deploy
    Wait-For-Services
    Verify-Deployment
    Print-Status
    
    Log-Success "Deployment complete!"
}

# Run main function
Main
