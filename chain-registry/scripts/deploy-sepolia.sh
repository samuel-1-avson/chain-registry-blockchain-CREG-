#!/bin/bash
set -e

# Chain Registry Sepolia Deployment Script
# Deploys Token and Staking contracts to Ethereum Sepolia.

# Check if .env.sepolia exists
if [ ! -f "testnet/.env.sepolia" ]; then
    echo "Error: testnet/.env.sepolia not found."
    exit 1
fi

# Source environment variables
export $(grep -v '^#' testnet/.env.sepolia | xargs)

if [ -z "$SEPOLIA_RPC_URL" ] || [ -z "$DEPLOYER_KEY" ]; then
    echo "Error: SEPOLIA_RPC_URL and DEPLOYER_KEY must be set in testnet/.env.sepolia"
    exit 1
fi

echo "🚀 Deploying to Sepolia..."
echo "RPC URL: $SEPOLIA_RPC_URL"

# Run deployment using forge inside the anvil container (which has forge installed)
docker exec -e DEPLOYER_KEY=$DEPLOYER_KEY creg-local-anvil forge script testnet/Deploy.s.sol:DeployScript --rpc-url $SEPOLIA_RPC_URL --broadcast --slow

# Check if manifest was created
if [ -f "testnet/artifacts/testnet-contracts.json" ]; then
    TOKEN_ADDR=$(jq -r '.token' testnet/artifacts/testnet-contracts.json)
    STAKING_ADDR=$(jq -r '.staking' testnet/artifacts/testnet-contracts.json)
    echo "✅ Deployment successful!"
    echo "Token: $TOKEN_ADDR"
    echo "Staking: $STAKING_ADDR"
    
    # Update .env.testnet for local reference if needed
    sed -i "s/^TESTNET_TOKEN_ADDR=.*/TESTNET_TOKEN_ADDR=$TOKEN_ADDR/" .env.testnet
    sed -i "s/^TESTNET_STAKING_ADDR=.*/TESTNET_STAKING_ADDR=$STAKING_ADDR/" .env.testnet
    
    echo "Please update your VITE_SEPOLIA_CREG_TOKEN and VITE_SEPOLIA_STAKING_ADDR in your environment."
else
    echo "❌ Deployment failed or manifest not found."
    exit 1
fi
