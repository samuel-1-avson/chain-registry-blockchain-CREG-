#!/bin/bash
# Deploy testnet contracts using Docker
set -e

RPC_URL="http://creg-testnet-anvil:8545"
DEPLOYER_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
FAUCET_ADDR="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

echo "=== Chain Registry Testnet Contract Deployment ==="
echo ""

# Deploy Test Token
echo "[1/4] Deploying Test CREG Token..."
forge create TestCregToken.sol:TestCregToken \
  --rpc-url "$RPC_URL" \
  --private-key "$DEPLOYER_KEY" \
  --constructor-args "Test CREG Token" "tCREG" \
  --json > /tmp/token.json 2>&1

TOKEN_ADDR=$(cat /tmp/token.json | grep -o '"deployedTo":"[^"]*"' | cut -d'"' -f4)
echo "Token deployed at: $TOKEN_ADDR"

# Deploy Test Staking
echo ""
echo "[2/4] Deploying Test Staking..."
forge create TestStaking.sol:TestStaking \
  --rpc-url "$RPC_URL" \
  --private-key "$DEPLOYER_KEY" \
  --constructor-args "$TOKEN_ADDR" \
  --json > /tmp/staking.json 2>&1

STAKING_ADDR=$(cat /tmp/staking.json | grep -o '"deployedTo":"[^"]*"' | cut -d'"' -f4)
echo "Staking deployed at: $STAKING_ADDR"

# Setup faucet
echo ""
echo "[3/4] Setting up faucet..."
cast send "$TOKEN_ADDR" "setFaucet(address)" "$FAUCET_ADDR" \
  --rpc-url "$RPC_URL" \
  --private-key "$DEPLOYER_KEY"
echo "Faucet configured"

# Mint tokens to faucet
echo ""
echo "[4/4] Minting 1,000,000 tCREG to faucet..."
cast send "$TOKEN_ADDR" "mint(address,uint256)" "$FAUCET_ADDR" 1000000000000000000000000 \
  --rpc-url "$RPC_URL" \
  --private-key "$DEPLOYER_KEY"
echo "Tokens minted"

# Save results
mkdir -p /workspace/testnet/artifacts
cat > /workspace/testnet/artifacts/testnet-contracts.json << EOF
{
  "network": "testnet",
  "chainId": 31337,
  "rpcUrl": "http://localhost:8545",
  "deployedAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "contracts": {
    "TestCregToken": {
      "address": "$TOKEN_ADDR",
      "name": "Test CREG Token",
      "symbol": "tCREG"
    },
    "TestStaking": {
      "address": "$STAKING_ADDR",
      "token": "$TOKEN_ADDR"
    }
  },
  "accounts": {
    "deployer": "$DEPLOYER_ADDR",
    "faucet": "$FAUCET_ADDR"
  }
}
EOF

cat > /workspace/testnet/artifacts/testnet.env << EOF
TESTNET_TOKEN_ADDR=$TOKEN_ADDR
TESTNET_STAKING_ADDR=$STAKING_ADDR
TESTNET_REGISTRY_ADDR=$STAKING_ADDR
TESTNET_RPC_URL=http://localhost:8545
TESTNET_CHAIN_ID=31337
FAUCET_URL=http://localhost:8081
FAUCET_ADDRESS=$FAUCET_ADDR
TESTNET_NODE_URL=http://localhost:8080
EOF

echo ""
echo "=== Deployment Complete ==="
echo "Token: $TOKEN_ADDR"
echo "Staking: $STAKING_ADDR"
