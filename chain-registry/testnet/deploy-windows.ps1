# Chain Registry Testnet Deployment Script
# Run this in PowerShell

$ErrorActionPreference = "Stop"

$RPC_URL = "http://localhost:8545"
$DEPLOYER_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
$DEPLOYER_ADDR = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
$FAUCET_ADDR = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "Chain Registry Contract Deployment" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Check Anvil is running
Write-Host "Checking Anvil connection..." -ForegroundColor Yellow
try {
    $response = Invoke-RestMethod -Uri "http://localhost:8545" -Method Post -ContentType "application/json" -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -TimeoutSec 5
    Write-Host "Anvil is running. Block: $($response.result)" -ForegroundColor Green
} catch {
    Write-Host "ERROR: Cannot connect to Anvil at http://localhost:8545" -ForegroundColor Red
    Write-Host "Make sure the testnet is running:" -ForegroundColor Yellow
    Write-Host "  docker-compose -f testnet/docker-compose.testnet.yml up -d anvil postgres" -ForegroundColor Gray
    exit 1
}

Write-Host ""
Write-Host "Ready to deploy!" -ForegroundColor Green
Write-Host ""
Write-Host "Run these commands one by one in your terminal:" -ForegroundColor Cyan
Write-Host ""
Write-Host "# 1. Deploy Test Token:" -ForegroundColor Yellow
Write-Host "cd F:\project\chain-registry\chain-registry" -ForegroundColor Gray
Write-Host "docker run --rm -v `${PWD}:/workspace -w /workspace/contracts/testnet --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c 'forge create TestCregToken.sol:TestCregToken --rpc-url http://creg-testnet-anvil:8545 --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --constructor-args \"Test CREG Token\" \"tCREG\"'"" -ForegroundColor White
Write-Host ""
Write-Host "# 2. After getting token address, deploy Staking:" -ForegroundColor Yellow
Write-Host "docker run --rm -v `${PWD}:/workspace -w /workspace/contracts/testnet --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c 'forge create TestStaking.sol:TestStaking --rpc-url http://creg-testnet-anvil:8545 --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --constructor-args <TOKEN_ADDRESS>'"" -ForegroundColor White
Write-Host ""
Write-Host "# 3. Setup faucet:" -ForegroundColor Yellow
Write-Host "docker run --rm --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c 'cast send <TOKEN_ADDRESS> `""setFaucet(address)`"" $FAUCET_ADDR --rpc-url http://creg-testnet-anvil:8545 --private-key $DEPLOYER_KEY'"" -ForegroundColor White
Write-Host ""
Write-Host "# 4. Mint tokens to faucet:" -ForegroundColor Yellow
Write-Host "docker run --rm --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c 'cast send <TOKEN_ADDRESS> `""mint(address,uint256)`"" $FAUCET_ADDR 1000000000000000000000000 --rpc-url http://creg-testnet-anvil:8545 --private-key $DEPLOYER_KEY'"" -ForegroundColor White
