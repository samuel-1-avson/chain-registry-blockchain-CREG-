#!/usr/bin/env bash
# ==============================================================================
# Chain Registry (CREG) — Independent External Validator Launch Script
# ==============================================================================
# This script enables ANY independent operator to join the decentralized CREG
# testnet as a full validator on their own server, cloud VM, or bare-metal host.
# ==============================================================================

set -euo pipefail

CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${CYAN}============================================================${NC}"
echo -e "${CYAN}   Chain Registry (CREG) — External Validator Onboarding    ${NC}"
echo -e "${CYAN}============================================================${NC}"

# Check Docker
if ! command -v docker >/dev/null 2>&1; then
    echo -e "${RED}Error: Docker is not installed. Please install Docker first.${NC}"
    exit 1
fi

DATA_DIR="${HOME}/.creg-validator"
mkdir -p "${DATA_DIR}"
ENV_FILE="${DATA_DIR}/validator.env"

# Generate or load Validator Key
if [ ! -f "${DATA_DIR}/validator.key" ]; then
    echo -e "${YELLOW}Generating new Ed25519 validator consensus key...${NC}"
    VALIDATOR_KEY=$(openssl rand -hex 32)
    echo "${VALIDATOR_KEY}" > "${DATA_DIR}/validator.key"
    chmod 600 "${DATA_DIR}/validator.key"
    echo -e "${GREEN}[OK] Validator key saved to ${DATA_DIR}/validator.key${NC}"
else
    VALIDATOR_KEY=$(cat "${DATA_DIR}/validator.key" | tr -d '\r\n ')
    echo -e "${GREEN}[OK] Loaded existing validator key from ${DATA_DIR}/validator.key${NC}"
fi

# Operator API Key
if [ ! -f "${DATA_DIR}/operator_api.key" ]; then
    OPERATOR_KEY=$(openssl rand -hex 32)
    echo "${OPERATOR_KEY}" > "${DATA_DIR}/operator_api.key"
    chmod 600 "${DATA_DIR}/operator_api.key"
else
    OPERATOR_KEY=$(cat "${DATA_DIR}/operator_api.key" | tr -d '\r\n ')
fi

NODE_ID="${NODE_ID:-ext-val-$(hostname -s 2>/dev/null || echo $RANDOM)}"
ETH_RPC="${ETH_RPC:-https://ethereum-sepolia-rpc.publicnode.com}"
BOOTNODE="/ip4/35.225.225.20/tcp/9000/p2p/12D3KooWDro5WW69YcAhFpqNSx9aGho3NdhU5LShjxSZjCFFE5XJ"

cat <<EOF > "${ENV_FILE}"
CREG_TESTNET=true
CREG_NODE_ID=${NODE_ID}
CREG_DATA_DIR=/data
CREG_IS_VALIDATOR=true
CREG_NODE_ROLE=validator
CREG_VALIDATOR_KEY=${VALIDATOR_KEY}
CREG_OPERATOR_API_KEY=${OPERATOR_KEY}
CREG_ETH_RPC=${ETH_RPC}
CREG_EXPECTED_L1_CHAIN_ID=11155111
CREG_CHAIN_ID=creg-testnet-1
CREG_CHAIN_SPEC_URL=https://spec.testnet.cregnet.dev/chain-spec.json
CREG_SPEC_SIGNATURE_URL=https://spec.testnet.cregnet.dev/chain-spec.json.sig
CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev
CREG_P2P_LISTEN=/ip4/0.0.0.0/tcp/9000
CREG_P2P_SEEDS=${BOOTNODE}
CREG_LISTEN=0.0.0.0:8080
RUST_LOG=info,chain_registry_node=debug,consensus=info
EOF

echo -e "\n${YELLOW}Starting independent validator container...${NC}"

docker run -d \
    --name "creg-validator-${NODE_ID}" \
    --restart unless-stopped \
    --env-file "${ENV_FILE}" \
    -p 8080:8080 \
    -p 9000:9000 \
    -v "${DATA_DIR}/chain-data:/data" \
    creg-node:local-3node /app/creg-node

echo -e "\n${GREEN}============================================================${NC}"
echo -e "${GREEN}   Validator Node Successfully Launched!                    ${NC}"
echo -e "${GREEN}============================================================${NC}"
echo -e "Node ID:       ${CYAN}${NODE_ID}${NC}"
echo -e "Data Dir:      ${CYAN}${DATA_DIR}${NC}"
echo -e "Bootnode Peer: ${CYAN}${BOOTNODE}${NC}"
echo -e "Local RPC:     ${CYAN}http://localhost:8080/rpc${NC}"
echo -e "Local Health:  ${CYAN}http://localhost:8080/v1/health${NC}"
echo -e "\n${YELLOW}To participate in voting and receive block rewards:${NC}"
echo -e "1. Get free test tokens: ${CYAN}https://faucet.testnet.cregnet.dev${NC}"
echo -e "2. Stake 100 tCREG on Sepolia contract: ${CYAN}0x30604Aa6f763fbde6aDCffbd8960af3031037331${NC}"
echo -e "3. Monitor your node logs: ${CYAN}docker logs -f creg-validator-${NODE_ID}${NC}\n"
