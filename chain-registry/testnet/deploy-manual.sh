#!/bin/bash
set -e

cd /workspace/contracts/testnet

echo "Deploying Test CREG Token..."
forge create TestCregToken.sol:TestCregToken \
  --rpc-url http://creg-testnet-anvil:8545 \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --constructor-args "Test CREG Token" "tCREG" \
  2>&1 | tee /workspace/testnet/artifacts/token-deploy.log

echo ""
echo "Token deployment complete. Check token-deploy.log for address."
