# Validator Key Generation Script for Chain Registry (PowerShell)
#
# This script generates the validator key for Chain Registry testnet.

param(
    [int]$NumValidators = 1,
    [switch]$Help
)

if ($Help) {
    Write-Host @"
Validator Key Generation Script for Chain Registry

Usage:
    .\generate-validator-keys.ps1 [OPTIONS]

Options:
    -NumValidators <n>  Number of validators to generate (default: 1)
    -Help               Show this help

Examples:
    .\generate-validator-keys.ps1                    # Generate 1 validator
"@
    exit 0
}

# Colors (compatible with PowerShell 5.1)
$Red = "[31m"
$Green = "[32m"
$Yellow = "[33m"
$Blue = "[34m"
$NC = "[0m"

# Helper function for colored output
function Write-Color($Color, $Message) {
    Write-Host "$([char]27)$Color$Message$([char]27)$NC"
}

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Resolve-Path (Join-Path $ScriptDir "..")
$KeysDir = Join-Path $ProjectRoot "validator-keys"
$EnvFile = Join-Path $ProjectRoot ".env"

function Print-Header {
    Write-Host ""
    Write-Color $Blue "========================================"
    Write-Color $Blue "  Chain Registry - Validator Key Generator"
    Write-Color $Blue "========================================"
    Write-Host ""
}

function Print-ArchitectureNote {
    Write-Host ""
    Write-Color $Yellow "========================================"
    Write-Color $Yellow "  ARCHITECTURE NOTE:"
    Write-Color $Yellow "  PRODUCTION: One validator per PC ONLY"
    Write-Color $Yellow "  TESTING: Multiple validators on one PC is OK"
    Write-Color $Yellow "  This script is for TESTNET TESTING ONLY"
    Write-Color $Yellow "========================================"
    Write-Host ""
}

function Test-Dependencies {
    Write-Color $Blue "[INFO] Checking dependencies..."
    
    # Check if cargo is available
    try {
        $null = cargo --version 2>$null
        if ($LASTEXITCODE -ne 0) { throw "Cargo not found" }
    } catch {
        Write-Color $Red "[ERROR] Rust/Cargo not found. Please install Rust first."
        Write-Host "  Run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        exit 1
    }
    
    Write-Color $Green "[OK] Dependencies checked"
}

function Generate-Keys {
    Write-Color $Blue "[INFO] Generating $NumValidators validator key(s)..."
    
    # Create keys directory
    New-Item -ItemType Directory -Force -Path $KeysDir | Out-Null
    
    # Backup existing .env if present
    if (Test-Path $EnvFile) {
        $BackupFile = "$EnvFile.backup.$(Get-Date -Format 'yyyyMMdd_HHmmss')"
        Copy-Item $EnvFile $BackupFile
        Write-Color $Yellow "[WARN] Backed up existing .env file to $BackupFile"
    }
    
    # Create new .env from example if doesn't exist
    if (-not (Test-Path $EnvFile)) {
        $EnvExample = Join-Path $ProjectRoot ".env.example"
        if (Test-Path $EnvExample) {
            Copy-Item $EnvExample $EnvFile
        } else {
            New-Item -ItemType File -Path $EnvFile | Out-Null
        }
    }
    
    # Generate keys for each validator
    for ($i = 1; $i -le $NumValidators; $i++) {
        Write-Host ""
        Write-Host "$Blue[INFO]$NC Generating Validator $i key..."
        
        # Generate key using openssl
        $Bytes = New-Object byte[] 32
        [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($Bytes)
        $PrivateKey = ($Bytes | ForEach-Object { $_.ToString("x2") }) -join ""
        
        # Create validator env file
        $ValidatorEnvFile = Join-Path $KeysDir "validator-$i.env"
        @"
# Validator $i Configuration
# Generated: $(Get-Date -Format "o")

NODE${i}_VALIDATOR_KEY=$PrivateKey
NODE${i}_ID=node-$i
NODE${i}_DATA_DIR=./data/node-$i
"@ | Out-File -FilePath $ValidatorEnvFile -Encoding utf8
        
        # Update main .env file
        $Pattern = "NODE${i}_VALIDATOR_KEY="
        $Line = "NODE${i}_VALIDATOR_KEY=$PrivateKey"
        
        if (Test-Path $EnvFile) {
            $Content = Get-Content $EnvFile -Raw
            if ($Content -match "^$Pattern.*$") {
                # Update existing
                $Content = $Content -replace "^$Pattern.*$", $Line
                $Content | Out-File -FilePath $EnvFile -Encoding utf8
            } else {
                # Add new
                Add-Content -Path $EnvFile -Value ""
                Add-Content -Path $EnvFile -Value "# Validator $i"
                Add-Content -Path $EnvFile -Value $Line
            }
        }
        
        Write-Color $Green "[OK] Validator $i`: Key generated and saved"
        Write-Host "  Private: $([char]27)$Yellow$($PrivateKey.Substring(0,16))...$([char]27)$NC"
        Write-Host "  Config:  $ValidatorEnvFile"
    }
    
    Write-Host ""
    Write-Color $Green "[SUCCESS] Generated $NumValidators validator key(s)"
}

function Create-ValidatorConfigs {
    Write-Host ""
    Write-Color $Blue "[INFO] Creating validator configuration files..."
    
    for ($i = 1; $i -le $NumValidators; $i++) {
        $ConfigFile = Join-Path $KeysDir "validator-$i-docker-compose.yml"
                $ValidatorKeyRef = '${NODE' + $i + '_VALIDATOR_KEY}'
        
        # Calculate ports
        $ApiPort = 8080 + $i - 1
        $P2pPort = 9000 + $i - 1
        $GrpcPort = 50051 + $i - 1
        
                $ConfigContent = @'
# Validator __INDEX__ Docker Compose
# Generated automatically - DO NOT EDIT MANUALLY
# Run this file on exactly one validator host.

version: "3.9"

services:
    node-__INDEX__:
        build:
            context: ..
            dockerfile: ${CREG_DOCKERFILE:-Dockerfile}
        container_name: creg-validator-__INDEX__
        environment:
            CREG_NODE_ID: "node-__INDEX__"
            CREG_IS_VALIDATOR: "true"
            CREG_VALIDATOR_KEY: "__VALIDATOR_KEY_REF__"
            CREG_LISTEN: "0.0.0.0:8080"
            CREG_P2P_LISTEN: "/ip4/0.0.0.0/tcp/9000"
            CREG_DATA_DIR: "/data"
            CREG_SINGLE_VALIDATOR_MODE: "${CREG_SINGLE_VALIDATOR_MODE:-false}"
            CREG_DEV_SANDBOX: "${CREG_DEV_SANDBOX:-false}"
            CREG_ETH_RPC: "${CREG_ETH_RPC:?set CREG_ETH_RPC to the shared Anvil RPC URL}"
            CREG_IPFS_URL: "${CREG_IPFS_URL:?set CREG_IPFS_URL to the shared IPFS API URL}"
            CREG_PG_URL: "${CREG_PG_URL:-}"
            CREG_P2P_SEEDS: "${CREG_P2P_SEEDS:-}"
            CREG_VALIDATOR_SET: "${VALIDATOR_SET_JSON:-}"
            CREG_TOKEN_ADDR: "${TESTNET_TOKEN_ADDR:?set TESTNET_TOKEN_ADDR from the bootstrap host .env.testnet}"
            CREG_STAKING_ADDR: "${TESTNET_STAKING_ADDR:?set TESTNET_STAKING_ADDR from the bootstrap host .env.testnet}"
            CREG_REGISTRY_ADDR: "${TESTNET_REGISTRY_ADDR:?set TESTNET_REGISTRY_ADDR from the bootstrap host .env.testnet}"
            CREG_GOVERNANCE_ADDR: "${TESTNET_GOVERNANCE_ADDR:-}"
            CREG_BRIDGE_KEY: "${CREG_BRIDGE_KEY:-}"
            CREG_ZK_ENABLED: "${CREG_ZK_ENABLED:-true}"
            CREG_ML_ENABLED: "${CREG_ML_ENABLED:-true}"
            CREG_ML_MODEL_PATH: "/app/models"
            CREG_WASM_ENABLED: "${CREG_WASM_ENABLED:-true}"
            CREG_WASM_VALIDATORS_PATH: "/app/validators"
            CREG_TLS_CERT: "${CREG_TLS_CERT:-/app/certs/server.crt}"
            CREG_TLS_KEY: "${CREG_TLS_KEY:-/app/certs/server.key}"
            RUST_LOG: "info,chain_registry_node=debug"
        ports:
            - "__API_PORT__:8080"
            - "__GRPC_PORT__:50051"
            - "__P2P_PORT__:9000"
        volumes:
            - ../data/node-__INDEX__:/data
            - ../circuits:/app/circuits:ro
            - ../validators:/app/validators:ro
            - ../models:/app/models:ro
            - ../config/sandbox:/app/config/sandbox:ro
            - ../testnet/certs:/app/certs:ro
            - /var/run/docker.sock:/var/run/docker.sock
        restart: unless-stopped
'@
                $ConfigContent = $ConfigContent.Replace('__INDEX__', $i.ToString())
                $ConfigContent = $ConfigContent.Replace('__VALIDATOR_KEY_REF__', $ValidatorKeyRef)
                $ConfigContent = $ConfigContent.Replace('__API_PORT__', $ApiPort.ToString())
                $ConfigContent = $ConfigContent.Replace('__GRPC_PORT__', $GrpcPort.ToString())
                $ConfigContent = $ConfigContent.Replace('__P2P_PORT__', $P2pPort.ToString())
        $ConfigContent | Out-File -FilePath $ConfigFile -Encoding utf8
        
        Write-Color $Green "[OK] Validator $i`: Config created (API: $ApiPort, P2P: $P2pPort)"
    }
}

function Print-Summary {
    Write-Host ""
    Write-Color $Blue "========================================"
    Write-Color $Blue "  GENERATION COMPLETE"
    Write-Color $Blue "========================================"
    Write-Host ""
    Write-Host "Files created:"
    Write-Host "  [OK] $EnvFile (updated with validator keys)"
    Write-Host "  [OK] $KeysDir\validator-{1..$NumValidators}.env"
    Write-Host "  [OK] $KeysDir\validator-{1..$NumValidators}-docker-compose.yml"
    Write-Host ""
    Write-Host "Next steps:"
    Write-Host "  1. Start shared services on the bootstrap host: docker compose --env-file .env.testnet -f docker-compose.testnet.yml up -d ipfs postgres anvil deploy-contracts faucet web-explorer"
    Write-Host "  2. Copy validator-1.env and validator-1-docker-compose.yml to validator host 1"
    Write-Host "  3. Copy TESTNET_TOKEN_ADDR / TESTNET_STAKING_ADDR / TESTNET_REGISTRY_ADDR from the bootstrap host .env.testnet into that validator env file"
    Write-Host "  4. Set CREG_ETH_RPC / CREG_IPFS_URL / CREG_PG_URL / CREG_P2P_SEEDS in that validator env file"
    Write-Host "  5. Start validator 1: docker compose --env-file validator-keys/validator-1.env -f validator-keys/validator-1-docker-compose.yml up -d --build"
    if ($NumValidators -gt 1) {
        Write-Host "  6. Start other validators using their generated env + compose pairs"
    }
    Write-Host ""
    Write-Color $Yellow "REMINDER: This multi-validator setup is for TESTING ONLY"
    Write-Host "In production, run ONE validator per PC."
    Write-Host ""
}

# Main
Print-Header
Print-ArchitectureNote
Test-Dependencies
Generate-Keys
Create-ValidatorConfigs
Print-Summary
