# Chain Registry Testnet Deployment Script for Windows
param()

$RPC_URL = "http://localhost:8545"
$DEPLOYER_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
$DEPLOYER_ADDR = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
$FAUCET_ADDR = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

Write-Host "Chain Registry Testnet Deployment" -ForegroundColor Cyan

# Create artifacts directory
New-Item -ItemType Directory -Force -Path testnet/artifacts | Out-Null

# Step 1: Deploy Test Token
Write-Host "`n[1/4] Deploying Test CREG Token..." -ForegroundColor Yellow

$tokenOutput = docker run --rm -v "${PWD}:/workspace" -w /workspace/contracts/testnet --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c "forge create TestCregToken.sol:TestCregToken --rpc-url http://creg-testnet-anvil:8545 --private-key $DEPLOYER_KEY --constructor-args 'Test CREG Token' 'tCREG' --json" 2>&1

Write-Host "Output: $tokenOutput"

# Extract token address
if ($tokenOutput -match 'deployedTo.*(0x[a-fA-F0-9]{40})') {
    $TOKEN_ADDR = $matches[1]
    Write-Host "Token deployed at: $TOKEN_ADDR" -ForegroundColor Green
    $tokenOutput | Out-File -FilePath testnet/artifacts/token.json
} else {
    Write-Host "Token deployment may have failed. Check output above." -ForegroundColor Yellow
    $TOKEN_ADDR = Read-Host "Enter token address manually (or press Ctrl+C to exit)"
}

# Step 2: Deploy Test Staking
Write-Host "`n[2/4] Deploying Test Staking Contract..." -ForegroundColor Yellow

$stakingOutput = docker run --rm -v "${PWD}:/workspace" -w /workspace/contracts/testnet --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c "forge create TestStaking.sol:TestStaking --rpc-url http://creg-testnet-anvil:8545 --private-key $DEPLOYER_KEY --constructor-args $TOKEN_ADDR --json" 2>&1

Write-Host "Output: $stakingOutput"

if ($stakingOutput -match 'deployedTo.*(0x[a-fA-F0-9]{40})') {
    $STAKING_ADDR = $matches[1]
    Write-Host "Staking deployed at: $STAKING_ADDR" -ForegroundColor Green
    $stakingOutput | Out-File -FilePath testnet/artifacts/staking.json
} else {
    Write-Host "Staking deployment may have failed. Check output above." -ForegroundColor Yellow
    $STAKING_ADDR = Read-Host "Enter staking address manually (or press Ctrl+C to exit)"
}

# Step 3: Set faucet
Write-Host "`n[3/4] Setting up faucet..." -ForegroundColor Yellow
docker run --rm --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c "cast send $TOKEN_ADDR 'setFaucet(address)' $FAUCET_ADDR --rpc-url http://creg-testnet-anvil:8545 --private-key $DEPLOYER_KEY"

# Step 4: Mint tokens to faucet
Write-Host "`n[4/4] Minting tokens to faucet..." -ForegroundColor Yellow
docker run --rm --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c "cast send $TOKEN_ADDR 'mint(address,uint256)' $FAUCET_ADDR 1000000000000000000000000 --rpc-url http://creg-testnet-anvil:8545 --private-key $DEPLOYER_KEY"

# Save contract addresses
$contracts = @{
    network = "testnet"
    chainId = 31337
    rpcUrl = $RPC_URL
    deployedAt = (Get-Date -Format "o")
    contracts = @{
        TestCregToken = @{
            address = $TOKEN_ADDR
            name = "Test CREG Token"
            symbol = "tCREG"
        }
        TestStaking = @{
            address = $STAKING_ADDR
            token = $TOKEN_ADDR
        }
    }
}

$contracts | ConvertTo-Json -Depth 10 | Out-File -FilePath testnet/artifacts/testnet-contracts.json

# Save environment file
"TESTNET_TOKEN_ADDR=$TOKEN_ADDR`nTESTNET_STAKING_ADDR=$STAKING_ADDR`nTESTNET_RPC_URL=$RPC_URL" | Out-File -FilePath testnet/artifacts/testnet.env

Write-Host "`nDeployment Complete!" -ForegroundColor Green
Write-Host "Token: $TOKEN_ADDR"
Write-Host "Staking: $STAKING_ADDR"
