# Chain Registry Testnet Deployment Script (PowerShell version)
# Deploys test contracts and sets up the testnet environment

param(
    [string]$RpcUrl = "http://localhost:8545",
    [string]$DeployerKey = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    [switch]$Help
)

if ($Help) {
    Write-Host @"
Chain Registry Testnet Deployment Script

Usage:
    .\deploy-testnet.ps1 [OPTIONS]

Options:
    -RpcUrl <url>       RPC URL (default: http://localhost:8545)
    -DeployerKey <key>  Deployer private key (default: Anvil account #0)
    -Help               Show this help

Environment Variables:
    RPC_URL             Override RPC URL
    DEPLOYER_KEY        Override deployer key

Examples:
    .\deploy-testnet.ps1                                    # Use defaults
    .\deploy-testnet.ps1 -RpcUrl http://localhost:8545      # Custom RPC
"@
    exit 0
}

# Use environment variables if set
if ($env:RPC_URL) { $RpcUrl = $env:RPC_URL }
if ($env:DEPLOYER_KEY) { $DeployerKey = $env:DEPLOYER_KEY }

# Colors
$Red = "`e[31m"
$Green = "`e[32m"
$Yellow = "`e[33m"
$Blue = "`e[34m"
$NC = "`e[0m"

Write-Host "$Blue`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}$NC"
Write-Host "$Blue`u{2551}       Chain Registry Testnet Deployment                  $Blue`u{2551}$NC"
Write-Host "$Blue`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}$NC"

# Configuration
$DeployerAddr = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

# Faucet account (Anvil account #1)
$FaucetKey = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
$FaucetAddr = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

Write-Host ""
Write-Host "$Yellow Configuration:$NC"
Write-Host "  RPC URL: $RpcUrl"
Write-Host "  Deployer: $DeployerAddr"
Write-Host "  Faucet: $FaucetAddr"

# Change to project root
$scriptPath = Split-Path -Parent $MyInvocation.MyCommand.Path
if ($scriptPath) {
    Set-Location (Join-Path $scriptPath "..")
}

# Check if forge is installed
try {
    $null = forge --version 2>$null
    if ($LASTEXITCODE -ne 0) { throw "Foundry not found" }
} catch {
    Write-Host "$Red Error: Foundry (forge) is not installed$NC"
    Write-Host "Install from: https://getfoundry.sh"
    exit 1
}

# Check connection
Write-Host ""
Write-Host "$Yellow Checking Ethereum connection...$NC"
try {
    $blockNumber = cast block-number --rpc-url $RpcUrl 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $blockNumber) { throw "Connection failed" }
} catch {
    Write-Host "$Red Error: Cannot connect to Ethereum at $RpcUrl$NC"
    Write-Host "Make sure Anvil is running: anvil --fork-url <url> --block-time 2"
    exit 1
}

Write-Host "$Green`u{2713} Connected to Ethereum (block $blockNumber)$NC"

# Get deployer balance
try {
    $balance = cast balance $DeployerAddr --rpc-url $RpcUrl 2>$null
    Write-Host "$Green`u{2713} Deployer balance: $balance$NC"
} catch {
    Write-Warning "Could not get deployer balance"
}

# Deploy Test CREG Token
Write-Host ""
Write-Host "$Yellow Deploying Test CREG Token...$NC"

$tokenOutput = forge create contracts/testnet/TestCregToken.sol:TestCregToken `
    --rpc-url $RpcUrl `
    --private-key $DeployerKey `
    --constructor-args "Test CREG Token" "tCREG" `
    --json 2>$null

if ($LASTEXITCODE -ne 0 -or -not $tokenOutput) {
    Write-Error "Token deployment failed"
    exit 1
}

# Extract address from JSON output
$tokenAddr = ($tokenOutput | ConvertFrom-Json).deployedTo
if (-not $tokenAddr) {
    # Fallback: try regex extraction
    if ($tokenOutput -match '"deployedTo":"([^"]*)"') {
        $tokenAddr = $matches[1]
    }
}

if (-not $tokenAddr) {
    Write-Error "Could not extract token address from deployment output"
    Write-Host "Output: $tokenOutput"
    exit 1
}

Write-Host "$Green`u{2713} Token deployed at: $tokenAddr$NC"

# Deploy Test Staking Contract
Write-Host ""
Write-Host "$Yellow Deploying Test Staking Contract...$NC"

$stakingOutput = forge create contracts/testnet/TestStaking.sol:TestStaking `
    --rpc-url $RpcUrl `
    --private-key $DeployerKey `
    --constructor-args $tokenAddr `
    --json 2>$null

if ($LASTEXITCODE -ne 0 -or -not $stakingOutput) {
    Write-Error "Staking deployment failed"
    exit 1
}

$stakingAddr = ($stakingOutput | ConvertFrom-Json).deployedTo
if (-not $stakingAddr) {
    if ($stakingOutput -match '"deployedTo":"([^"]*)"') {
        $stakingAddr = $matches[1]
    }
}

if (-not $stakingAddr) {
    Write-Error "Could not extract staking address from deployment output"
    exit 1
}

Write-Host "$Green`u{2713} Staking deployed at: $stakingAddr$NC"

# Set faucet address on token contract
Write-Host ""
Write-Host "$Yellow Setting up faucet...$NC"

cast send $tokenAddr "setFaucet(address)" $FaucetAddr `
    --rpc-url $RpcUrl `
    --private-key $DeployerKey `
    --quiet 2>$null

if ($LASTEXITCODE -ne 0) {
    Write-Warning "Failed to set faucet address"
}

# Mint 1,000,000 tCREG to faucet
Write-Host "$Yellow Minting tokens to faucet...$NC"

cast send $tokenAddr "mint(address,uint256)" $FaucetAddr 1000000000000000000000000 `
    --rpc-url $RpcUrl `
    --private-key $DeployerKey `
    --quiet 2>$null

if ($LASTEXITCODE -ne 0) {
    Write-Warning "Failed to mint tokens to faucet"
} else {
    Write-Host "$Green`u{2713} Faucet funded with 1,000,000 tCREG$NC"
}

# Save contract addresses
New-Item -ItemType Directory -Force -Path "testnet/artifacts" | Out-Null

$deployedAt = (Get-Date -Format "yyyy-MM-ddTHH:mm:ssZ")

$contractsJson = @"
{
  "network": "testnet",
  "chainId": 31337,
  "rpcUrl": "$RpcUrl",
  "deployedAt": "$deployedAt",
  "contracts": {
    "TestCregToken": {
      "address": "$tokenAddr",
      "name": "Test CREG Token",
      "symbol": "tCREG"
    },
    "TestStaking": {
      "address": "$stakingAddr",
      "token": "$tokenAddr"
    }
  },
  "accounts": {
    "deployer": "$DeployerAddr",
    "faucet": "$FaucetAddr"
  }
}
"@

$contractsJson | Out-File -FilePath "testnet/artifacts/testnet-contracts.json" -Encoding utf8

# Export environment variables
$envContent = @"
# Chain Registry Testnet Environment
# Generated: $(Get-Date)

# Contract Addresses
TESTNET_TOKEN_ADDR=$tokenAddr
TESTNET_STAKING_ADDR=$stakingAddr
TESTNET_REGISTRY_ADDR=$stakingAddr

# Connection
TESTNET_RPC_URL=$RpcUrl
TESTNET_CHAIN_ID=31337

# Faucet
FAUCET_URL=http://localhost:8081
FAUCET_ADDRESS=$FaucetAddr

# Node URLs
TESTNET_NODE_URL=http://localhost:8080
TESTNET_EXPLORER_URL=http://localhost:3000
"@

$envContent | Out-File -FilePath "testnet/artifacts/testnet.env" -Encoding utf8

Write-Host ""
Write-Host "$Green`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}$NC"
Write-Host "$Green`u{2551}       Testnet Deployment Complete!                       $Green`u{2551}$NC"
Write-Host "$Green`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}`u{2550}$NC"

Write-Host ""
Write-Host "$Blue Contract Addresses:$NC"
Write-Host "  Token:   $tokenAddr"
Write-Host "  Staking: $stakingAddr"

Write-Host ""
Write-Host "$Blue Environment Variables:$NC"
Write-Host "  `$env:TESTNET_TOKEN_ADDR = '$tokenAddr'"
Write-Host "  `$env:TESTNET_STAKING_ADDR = '$stakingAddr'"
Write-Host "  `$env:TESTNET_REGISTRY_ADDR = '$stakingAddr'"

Write-Host ""
Write-Host "$Blue Usage:$NC"
Write-Host "  1. Get test tokens:   Invoke-RestMethod -Uri 'http://localhost:8081/api/drip' -Method Post -Headers @{'Content-Type'='application/json'} -Body '{`"address`":`"YOUR_ADDRESS`"}'"
Write-Host "  2. Stake tokens:      cast send $stakingAddr 'stakeAsPublisher(uint256)' 1000000000000000000 --rpc-url $RpcUrl --private-key YOUR_KEY"
Write-Host "  3. Start validator:   `$env:CREG_IS_VALIDATOR='true'; `$env:CREG_VALIDATOR_KEY='...'; creg-node"

Write-Host ""
Write-Host "$Blue Files saved to:$NC"
Write-Host "  - testnet/artifacts/testnet-contracts.json"
Write-Host "  - testnet/artifacts/testnet.env"
