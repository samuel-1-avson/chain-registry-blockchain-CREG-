#!/usr/bin/env bash
# scripts/deploy_sepolia.sh
# Automated Sepolia Testnet Contract Deployment Script for Chain Registry

set -e

RPC_URL="${CREG_SEPOLIA_RPC_URL:-https://rpc.sepolia.org}"
PRIVATE_KEY="${CREG_DEPLOYER_PRIVATE_KEY:-}"
ETHERSCAN_API_KEY="${ETHERSCAN_API_KEY:-}"

echo "================================================================================"
echo "  CHAIN REGISTRY SEPOLIA TESTNET DEPLOYMENT"
echo "================================================================================"
echo "  RPC URL:       $RPC_URL"
echo "  Network:       Ethereum Sepolia (Chain ID: 11155111)"
echo ""

if [ -z "$PRIVATE_KEY" ]; then
    echo "⚠️  CREG_DEPLOYER_PRIVATE_KEY environment variable not set."
    echo "  To broadcast on-chain: export CREG_DEPLOYER_PRIVATE_KEY='0x...' and rerun."
    echo "  Running dry-run simulation using Foundry forge..."
    forge build --root contracts
    echo ""
    echo "✓ Contract bytecode & ABI compiled successfully."
    exit 0
fi

echo "► Deploying CrossChainRegistry.sol to Sepolia..."
VERIFY_FLAG=""
if [ -n "$ETHERSCAN_API_KEY" ]; then
    VERIFY_FLAG="--verify --etherscan-api-key $ETHERSCAN_API_KEY"
fi

forge create contracts/CrossChainRegistry.sol:CrossChainRegistry \
    --rpc-url "$RPC_URL" \
    --private-key "$PRIVATE_KEY" \
    $VERIFY_FLAG

echo ""
echo "✓ Sepolia deployment script finished."
