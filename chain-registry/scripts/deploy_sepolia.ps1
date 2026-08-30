# scripts/deploy_sepolia.ps1
# Automated Sepolia Testnet Contract Deployment Script for Chain Registry

param (
    [string]$RpcUrl = $env:CREG_SEPOLIA_RPC_URL,
    [string]$PrivateKey = $env:CREG_DEPLOYER_PRIVATE_KEY,
    [string]$EtherscanApiKey = $env:ETHERSCAN_API_KEY
)

if (-not $RpcUrl) {
    $RpcUrl = "https://rpc.sepolia.org"
}

Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "  CHAIN REGISTRY SEPOLIA TESTNET DEPLOYMENT" -ForegroundColor Cyan
Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "  RPC URL:       $RpcUrl" -ForegroundColor Yellow
Write-Host "  Network:       Ethereum Sepolia (Chain ID: 11155111)" -ForegroundColor Yellow
Write-Host ""

if (-not $PrivateKey) {
    Write-Host "⚠️ CREG_DEPLOYER_PRIVATE_KEY environment variable not set." -ForegroundColor Yellow
    Write-Host "  To broadcast on-chain: set `$env:CREG_DEPLOYER_PRIVATE_KEY='0x...' and rerun." -ForegroundColor Yellow
    Write-Host "  Running dry-run simulation using Foundry forge..." -ForegroundColor Cyan
    forge build --root contracts
    Write-Host "`n✓ Contract bytecode & ABI compiled successfully." -ForegroundColor Green
    exit 0
}

Write-Host "► Deploying CrossChainRegistry.sol to Sepolia..." -ForegroundColor Green
$verifyFlag = ""
if ($EtherscanApiKey) {
    $verifyFlag = "--verify --etherscan-api-key $EtherscanApiKey"
}

forge create contracts/CrossChainRegistry.sol:CrossChainRegistry `
    --rpc-url $RpcUrl `
    --private-key $PrivateKey `
    $verifyFlag

Write-Host "`n✓ Sepolia deployment script finished." -ForegroundColor Green
