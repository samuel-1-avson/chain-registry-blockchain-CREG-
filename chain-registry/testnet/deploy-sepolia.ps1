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

$toolsDir = Join-Path $scriptDir ".tools\foundry"
$toolsCast = Join-Path $toolsDir "cast.exe"
$toolsForge = Join-Path $toolsDir "forge.exe"
$cast = if (Test-Path $toolsCast) { $toolsCast } elseif (Get-Command cast -ErrorAction SilentlyContinue) { (Get-Command cast).Source } else { $null }
$forge = if (Test-Path $toolsForge) { $toolsForge } elseif (Get-Command forge -ErrorAction SilentlyContinue) { (Get-Command forge).Source } else { $null }
if (-not $cast -or -not $forge) {
    Write-Error "cast/forge not found. Run .\testnet\install-foundry.ps1 or add Foundry to PATH."
}
$env:FOUNDRY_DISABLE_NIGHTLY_WARNING = "1"

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
if (-not $env:DEPLOYER_KEY -and $env:GOVERNANCE_SIGNER_KEY) {
    $env:DEPLOYER_KEY = $env:GOVERNANCE_SIGNER_KEY
    Write-Host "Using GOVERNANCE_SIGNER_KEY as DEPLOYER_KEY (unified authority)." -ForegroundColor DarkGray
}
if (-not $env:DEPLOYER_KEY) {
    Write-Error "DEPLOYER_KEY is not set. Run .\testnet\setup-sepolia-authority.ps1"
}
if (-not $env:GOVERNANCE_THRESHOLD) {
    $env:GOVERNANCE_THRESHOLD = "1"
}
if ([int]$env:GOVERNANCE_THRESHOLD -le 1) {
    Write-Warning ("GOVERNANCE_THRESHOLD=1: a single signer (the bridge/deployer key) can propose " +
        "AND execute any governance action, including L1 anchoring, minting, and slashing. " +
        "Acceptable for a coordinated single-operator testnet only. Before public exposure, " +
        "redeploy with GOVERNANCE_THRESHOLD>=2 and independent GENESIS_SIGNERS.")
}
if ($env:GOVERNANCE_SIGNER_ADDRESS) {
    $derivedRaw = & $cast wallet address --private-key $env:DEPLOYER_KEY 2>&1 | Out-String
    $derived = if ($derivedRaw -match '(0x[a-fA-F0-9]{40})') { $matches[1] } else { $null }
    if ($derived -and ($derived.ToLower() -ne $env:GOVERNANCE_SIGNER_ADDRESS.Trim().ToLower())) {
        Write-Error "DEPLOYER_KEY does not match GOVERNANCE_SIGNER_ADDRESS. Run .\testnet\setup-sepolia-authority.ps1"
    }
}

# Safety check: refuse Anvil default key
$anvilKey = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
if ($env:DEPLOYER_KEY -eq $anvilKey) {
    Write-Error "DEPLOYER_KEY is the well-known Anvil default key. Generate a fresh key with: cast wallet new"
}

# Check deployer balance
$deployerRaw = & $cast wallet address --private-key $env:DEPLOYER_KEY 2>&1 | Out-String
if ($deployerRaw -notmatch '(0x[a-fA-F0-9]{40})') { Write-Error "Could not derive deployer address from DEPLOYER_KEY" }
$deployerAddr = $matches[1]
Write-Host "Deployer address: $deployerAddr"

$balance = & $cast balance $deployerAddr --rpc-url $env:SEPOLIA_RPC_URL 2>&1 | Out-String
$balance = $balance.Trim()
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

& $forge $forgeArgs

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
    & $forge $verifyArgs
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
