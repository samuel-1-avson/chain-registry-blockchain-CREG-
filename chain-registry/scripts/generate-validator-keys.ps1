# Validator Key Generation Script for Chain Registry (PowerShell)
#
# IMPORTANT ARCHITECTURE NOTE:
# ============================
# In PRODUCTION: Each validator MUST run on a separate PC
#   - One validator per physical/virtual machine
#   - Each with its own unique network identity
#   - For decentralization and security
#
# In TESTING: You CAN run multiple validators on one PC
#   - This script helps set up multiple test validators
#   - Each validator gets its own key and data directory
#   - Useful for testing consensus locally
#
# This script is for TESTNET TESTING ONLY

param(
    [int]$NumValidators = 3,
    [switch]$Help
)

if ($Help) {
    Write-Host @"
Validator Key Generation Script for Chain Registry

Usage:
    .\generate-validator-keys.ps1 [OPTIONS]

Options:
    -NumValidators <n>  Number of validators to generate (default: 3)
    -Help               Show this help

Examples:
    .\generate-validator-keys.ps1                    # Generate 3 validators
    .\generate-validator-keys.ps1 -NumValidators 5   # Generate 5 validators

Note: This script is for TESTNET TESTING ONLY.
In production, run ONE validator per PC.
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
        
        # Calculate ports
        $ApiPort = 8080 + $i - 1
        $P2pPort = 9000 + $i - 1
        $GrpcPort = 50051 + $i - 1
        
        # Build docker-compose content with proper escaping
        $ConfigContent = @"
# Validator $i Docker Compose
# Generated automatically - DO NOT EDIT MANUALLY

version: "3.9"

services:
  node-$i`:
    build:
      context: ..
      dockerfile: Dockerfile.minimal
    container_name: creg-validator-$i
    environment:
      CREG_NODE_ID: "node-$i"
      CREG_IS_VALIDATOR: "true"
      CREG_VALIDATOR_KEY: "`${NODE$i`_VALIDATOR_KEY}"
      CREG_LISTEN: "0.0.0.0:$ApiPort"
      CREG_API_PORT: "$ApiPort"
      CREG_GRPC_PORT: "$GrpcPort"
      CREG_P2P_LISTEN: "/ip4/0.0.0.0/tcp/$P2pPort"
      CREG_DATA_DIR: "/data"
      CREG_SINGLE_VALIDATOR_MODE: "false"
      CREG_DEV_SANDBOX: "true"
      CREG_ETH_RPC: "http://anvil:8545"
      RUST_LOG: "info,chain_registry_node=debug"
    ports:
      - "$ApiPort`:$ApiPort"
      - "$GrpcPort`:$GrpcPort"
      - "$P2pPort`:$P2pPort"
    volumes:
      - ../data/node-$i`:/data
    networks:
      - creg-network
    depends_on:
      - anvil
      - ipfs
    restart: unless-stopped

networks:
  creg-network:
    external: true
    name: chain-registry_creg-network
"@
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
    Write-Host "  1. Review .env file"
    Write-Host "  2. Start infrastructure: docker-compose up -d anvil ipfs"
    Write-Host "  3. Start validator 1: docker-compose up -d node"
    if ($NumValidators -gt 1) {
        Write-Host "  4. Start other validators using their compose files"
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
