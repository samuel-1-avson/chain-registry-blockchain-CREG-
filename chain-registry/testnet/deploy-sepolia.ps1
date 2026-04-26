# Chain Registry Sepolia Deployment Script (PowerShell)
# Prerequisites:
#   - forge/cast installed and on PATH
#   - .env.sepolia configured with SEPOLIA_RPC_URL and DEPLOYER_KEY
#
# Usage:
#   .\testnet\deploy-sepolia.ps1

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

# Load .env.sepolia
$envFile = Join-Path $scriptDir ".env.sepolia"
if (-not (Test-Path $envFile)) {
    Write-Error ".env.sepolia not found. Copy .env.sepolia.example and fill in your values."
}

Get-Content $envFile | ForEach-Object {
    if ($_ -match '^\s*([^#\s][^=]*)\s*=\s*(.*)\s*$') {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
    }
}

# Validate
if (-not $env:SEPOLIA_RPC_URL -or $env:SEPOLIA_RPC_URL -like "*YOUR_*") {
    Write-Error "SEPOLIA_RPC_URL is not set or still contains placeholder. Edit .env.sepolia"
}
if (-not $env:DEPLOYER_KEY) {
    Write-Error "DEPLOYER_KEY is not set. Edit .env.sepolia"
}

# Safety check: refuse Anvil default key
$anvilKey = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
if ($env:DEPLOYER_KEY -eq $anvilKey) {
    Write-Error "DEPLOYER_KEY is the well-known Anvil default key. Generate a fresh key with: cast wallet new"
}

# Check deployer balance
$deployerAddr = cast wallet address $env:DEPLOYER_KEY
Write-Host "Deployer address: $deployerAddr"

$balance = cast balance $deployerAddr --rpc-url $env:SEPOLIA_RPC_URL 2>$null
if ($LASTEXITCODE -ne 0) {
    $balance = "0"
}
Write-Host "Deployer balance: $balance"

if ($balance -eq "0") {
    Write-Error @"
Deployer balance is zero. Fund this address with Sepolia ETH before deploying:
  Address: $deployerAddr

Faucets:
  - https://sepolia-faucet.pk910.de (PoW mining faucet)
  - https://www.alchemy.com/faucets/ethereum-sepolia (requires Alchemy account)
  - https://www.infura.io/faucet/sepolia (requires Infura account)
"@
}

Write-Host ""
Write-Host "=== Chain Registry Sepolia Deployment ===" -ForegroundColor Cyan

# Run deployment
$forgeArgs = @(
    "script", "contracts/script/DeploySepolia.s.sol:DeploySepolia",
    "--rpc-url", $env:SEPOLIA_RPC_URL,
    "--private-key", $env:DEPLOYER_KEY,
    "--broadcast",
    "--chain-id", "11155111",
    "-vvv"
)

& forge $forgeArgs

if ($LASTEXITCODE -ne 0) {
    Write-Error "Deployment failed"
}

# Verify (optional)
if ($env:ETHERSCAN_API_KEY) {
    Write-Host "Verifying contracts on Etherscan..." -ForegroundColor Cyan
    $verifyArgs = @(
        "script", "contracts/script/DeploySepolia.s.sol:DeploySepolia",
        "--rpc-url", $env:SEPOLIA_RPC_URL,
        "--private-key", $env:DEPLOYER_KEY,
        "--verify",
        "--etherscan-api-key", $env:ETHERSCAN_API_KEY,
        "--chain-id", "11155111",
        "--resume",
        "-vvv"
    )
    & forge $verifyArgs
}

# Read deployment manifest
$manifestPath = Join-Path $repoRoot "contracts" "deployments" "sepolia-latest.json"
if (Test-Path $manifestPath) {
    Write-Host ""
    Write-Host "=== Deployment Manifest ===" -ForegroundColor Green
    Get-Content $manifestPath | ConvertFrom-Json | ConvertTo-Json -Depth 5
    Write-Host ""
    Write-Host "Manifest saved to: $manifestPath" -ForegroundColor Green
}

Write-Host ""
Write-Host "Next steps:" -ForegroundColor Cyan
Write-Host "  1. Copy contract addresses into testnet/chain-spec.json"
Write-Host "  2. Run: cargo run --example sign_chain_spec -- testnet/chain-spec.json <privkey_hex>"
Write-Host "  3. Publish chain-spec.json + signature to your spec server"
