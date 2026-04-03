#!/bin/bash
# Chain Registry Testnet Deployment Script
# Deploys test contracts and sets up the testnet environment

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       Chain Registry Testnet Deployment                  ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════════════════════╝${NC}"

# Configuration
RPC_URL="${RPC_URL:-http://localhost:8545}"
DEPLOYER_KEY="${DEPLOYER_KEY:-0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80}"
DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

# Faucet account (Anvil account #1)
FAUCET_KEY="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
FAUCET_ADDR="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

echo -e "\n${YELLOW}Configuration:${NC}"
echo "  RPC URL: $RPC_URL"
echo "  Deployer: $DEPLOYER_ADDR"
echo "  Faucet: $FAUCET_ADDR"

# Check if forge is installed
if ! command -v forge &> /dev/null; then
    echo -e "${RED}Error: Foundry (forge) is not installed${NC}"
    echo "Install from: https://getfoundry.sh"
    exit 1
fi

# Check connection
echo -e "\n${YELLOW}Checking Ethereum connection...${NC}"
if ! cast block-number --rpc-url "$RPC_URL" &> /dev/null; then
    echo -e "${RED}Error: Cannot connect to Ethereum at $RPC_URL${NC}"
    echo "Make sure Anvil is running: anvil --fork-url <url> --block-time 2"
    exit 1
fi

BLOCK_NUMBER=$(cast block-number --rpc-url "$RPC_URL")
echo -e "${GREEN}✓ Connected to Ethereum (block $BLOCK_NUMBER)${NC}"

# Get deployer balance
BALANCE=$(cast balance "$DEPLOYER_ADDR" --rpc-url "$RPC_URL")
echo -e "${GREEN}✓ Deployer balance: $BALANCE${NC}"

# Deploy Test CREG Token
echo -e "\n${YELLOW}Deploying Test CREG Token...${NC}"
TOKEN_OUTPUT=$(forge create contracts/testnet/TestCregToken.sol:TestCregToken \
    --rpc-url "$RPC_URL" \
    --private-key "$DEPLOYER_KEY" \
    --constructor-args "Test CREG Token" "tCREG" \
    --json 2>/dev/null)

TOKEN_ADDR=$(echo "$TOKEN_OUTPUT" | grep -o '"deployedTo":"[^"]*"' | cut -d'"' -f4)
echo -e "${GREEN}✓ Token deployed at: $TOKEN_ADDR${NC}"

# Deploy Test Staking Contract
echo -e "\n${YELLOW}Deploying Test Staking Contract...${NC}"
STAKING_OUTPUT=$(forge create contracts/testnet/TestStaking.sol:TestStaking \
    --rpc-url "$RPC_URL" \
    --private-key "$DEPLOYER_KEY" \
    --constructor-args "$TOKEN_ADDR" \
    --json 2>/dev/null)

STAKING_ADDR=$(echo "$STAKING_OUTPUT" | grep -o '"deployedTo":"[^"]*"' | cut -d'"' -f4)
echo -e "${GREEN}✓ Staking deployed at: $STAKING_ADDR${NC}"

# Set faucet address on token contract
echo -e "\n${YELLOW}Setting up faucet...${NC}"
cast send "$TOKEN_ADDR" "setFaucet(address)" "$FAUCET_ADDR" \
    --rpc-url "$RPC_URL" \
    --private-key "$DEPLOYER_KEY" \
    --quiet

# Mint 1,000,000 tCREG to faucet
echo -e "${YELLOW}Minting tokens to faucet...${NC}"
cast send "$TOKEN_ADDR" "mint(address,uint256)" "$FAUCET_ADDR" 1000000000000000000000000 \
    --rpc-url "$RPC_URL" \
    --private-key "$DEPLOYER_KEY" \
    --quiet

echo -e "${GREEN}✓ Faucet funded with 1,000,000 tCREG${NC}"

# Save contract addresses
mkdir -p testnet/artifacts
cat > testnet/artifacts/testnet-contracts.json << EOF
{
  "network": "testnet",
  "chainId": 31337,
  "rpcUrl": "$RPC_URL",
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

# Export environment variables
cat > testnet/artifacts/testnet.env << EOF
# Chain Registry Testnet Environment
# Generated: $(date)

# Contract Addresses
TESTNET_TOKEN_ADDR=$TOKEN_ADDR
TESTNET_STAKING_ADDR=$STAKING_ADDR
TESTNET_REGISTRY_ADDR=$STAKING_ADDR

# Connection
TESTNET_RPC_URL=$RPC_URL
TESTNET_CHAIN_ID=31337

# Faucet
FAUCET_URL=http://localhost:8081
FAUCET_ADDRESS=$FAUCET_ADDR

# Node URLs
TESTNET_NODE_URL=http://localhost:8080
TESTNET_EXPLORER_URL=http://localhost:3000
EOF

echo -e "\n${GREEN}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║       Testnet Deployment Complete!                       ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════════════════╝${NC}"

echo -e "\n${BLUE}Contract Addresses:${NC}"
echo "  Token:   $TOKEN_ADDR"
echo "  Staking: $STAKING_ADDR"

echo -e "\n${BLUE}Environment Variables:${NC}"
echo "  export TESTNET_TOKEN_ADDR=$TOKEN_ADDR"
echo "  export TESTNET_STAKING_ADDR=$STAKING_ADDR"
echo "  export TESTNET_REGISTRY_ADDR=$STAKING_ADDR"

echo -e "\n${BLUE}Usage:${NC}"
echo "  1. Get test tokens:   curl -X POST http://localhost:8081/api/drip \\"
echo "                        -H 'Content-Type: application/json' \\"
echo "                        -d '{\"address\":\"YOUR_ADDRESS\"}'"
echo "  2. Stake tokens:      cast send $STAKING_ADDR \\"
echo "                        'stakeAsPublisher(uint256)' 1000000000000000000 \\"
echo "                        --rpc-url $RPC_URL --private-key YOUR_KEY"
echo "  3. Start validator:   CREG_IS_VALIDATOR=true CREG_VALIDATOR_KEY=... creg-node"

echo -e "\n${BLUE}Files saved to:${NC}"
echo "  - testnet/artifacts/testnet-contracts.json"
echo "  - testnet/artifacts/testnet.env"
