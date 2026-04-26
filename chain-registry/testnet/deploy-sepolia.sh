#!/usr/bin/env bash
# testnet/deploy-sepolia.sh
# Deploy the Chain Registry contract suite to Ethereum Sepolia.
#
# Prerequisites:
#   - forge (Foundry) installed
#   - SEPOLIA_RPC_URL set (e.g. https://sepolia.infura.io/v3/YOUR_KEY)
#   - DEPLOYER_KEY set (fresh secp256k1 private key, NOT Anvil default)
#   - ETHERSCAN_API_KEY set (optional, for contract verification)
#
# Usage:
#   export SEPOLIA_RPC_URL=https://sepolia.infura.io/v3/...
#   export DEPLOYER_KEY=0x...
#   export CREG_BRIDGE_KEY=0x...     # optional: separate bridge/validator key
#   ./testnet/deploy-sepolia.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.."

# ── Validate environment ──
: "${SEPOLIA_RPC_URL:?Environment variable SEPOLIA_RPC_URL must be set}"
: "${DEPLOYER_KEY:?Environment variable DEPLOYER_KEY must be set}"

# Safety check: refuse Anvil default key
ANVIL_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
if [ "$DEPLOYER_KEY" = "$ANVIL_KEY" ]; then
    echo "ERROR: DEPLOYER_KEY is the well-known Anvil default key."
    echo "Generate a fresh key with: cast wallet new"
    exit 1
fi

# Fund check
DEPLOYER_ADDR=$(cast wallet address "$DEPLOYER_KEY")
echo "Deployer address: $DEPLOYER_ADDR"
BALANCE=$(cast balance "$DEPLOYER_ADDR" --rpc-url "$SEPOLIA_RPC_URL" 2>/dev/null || echo "0")
echo "Deployer balance: $BALANCE"

# ── Run deployment ──
forge script contracts/script/DeploySepolia.s.sol:DeploySepolia \
  --rpc-url "$SEPOLIA_RPC_URL" \
  --private-key "$DEPLOYER_KEY" \
  --broadcast \
  --chain-id 11155111 \
  -vvv

# ── Verify (optional) ──
if [ -n "${ETHERSCAN_API_KEY:-}" ]; then
    echo "Verifying contracts on Etherscan..."
    forge script contracts/script/DeploySepolia.s.sol:DeploySepolia \
      --rpc-url "$SEPOLIA_RPC_URL" \
      --private-key "$DEPLOYER_KEY" \
      --verify \
      --etherscan-api-key "$ETHERSCAN_API_KEY" \
      --chain-id 11155111 \
      --resume \
      -vvv
else
    echo "Skipping verification (set ETHERSCAN_API_KEY to verify)"
fi

echo ""
echo "Sepolia deployment complete."
echo "Manifest: contracts/deployments/sepolia-latest.json"
echo ""
echo "Next steps:"
echo "  1. Copy contract addresses into your chain-spec.json"
echo "  2. Set CREG_ETH_RPC=$SEPOLIA_RPC_URL"
echo "  3. Set CREG_EXPECTED_L1_CHAIN_ID=11155111"
