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

NODE_ID="${NODE_ID:-ext-val-$(hostname -s 2>/dev/null || echo $RANDOM)}"
ETH_RPC="${ETH_RPC:-https://ethereum-sepolia-rpc.publicnode.com}"
HTTP_PORT="${HTTP_PORT:-8080}"
P2P_PORT="${P2P_PORT:-9000}"
CUSTOM_KEY="${CUSTOM_KEY:-}"

# Isolated data dir per node ID to allow running multiple nodes on one host
if [ "${NODE_ID}" = "ext-val" ]; then
    DATA_DIR="${HOME}/.creg-validator"
else
    DATA_DIR="${HOME}/.creg-validator-${NODE_ID}"
fi
mkdir -p "${DATA_DIR}"
ENV_FILE="${DATA_DIR}/validator.env"

# Generate, load, or use custom Validator Key
if [ -n "${CUSTOM_KEY}" ]; then
    VALIDATOR_KEY=$(echo "${CUSTOM_KEY}" | tr -d '\r\n ' | tr '[:upper:]' '[:lower:]')
    echo "${VALIDATOR_KEY}" > "${DATA_DIR}/validator.key"
    chmod 600 "${DATA_DIR}/validator.key"
    echo -e "${GREEN}[OK] Using custom validator key (saved to ${DATA_DIR}/validator.key)${NC}"
elif [ ! -f "${DATA_DIR}/validator.key" ]; then
    echo -e "${YELLOW}Generating new Ed25519 validator consensus key...${NC}"
    VALIDATOR_KEY=$(openssl rand -hex 32)
    echo "${VALIDATOR_KEY}" > "${DATA_DIR}/validator.key"
    chmod 600 "${DATA_DIR}/validator.key"
    echo -e "${GREEN}[OK] Generated new validator key: ${DATA_DIR}/validator.key${NC}"
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

BOOTNODE="/ip4/35.225.225.20/tcp/9000/p2p/12D3KooWHrfvaBbsQmXq1L3ZAjmX299ca9pLECT5a2GdxaGGizgY"

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
CREG_STAKING_ADDR=0x30604Aa6f763fbde6aDCffbd8960af3031037331
CREG_REGISTRY_ADDR=0xD0F465ca185a99E268be0C4FF5A135Bdf89FAF3B
CREG_CHAIN_SPEC_URL=https://spec.testnet.cregnet.dev/chain-spec.json
CREG_SPEC_SIGNATURE_URL=https://spec.testnet.cregnet.dev/chain-spec.json.sig
CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev
CREG_P2P_LISTEN=/ip4/0.0.0.0/tcp/9000
CREG_P2P_SEEDS=${BOOTNODE}
CREG_LISTEN=0.0.0.0:8080
RUST_LOG=info,chain_registry_node=debug,consensus=info
EOF

# Auto-detect available Docker image
CHOSEN_IMAGE=""
for candidate in chain-registry-node-secure:latest creg-node:fleet creg-node:latest creg-node:local-3node chain-registry-app:latest; do
    if docker image inspect "${candidate}" >/dev/null 2>&1; then
        CHOSEN_IMAGE="${candidate}"
        break
    fi
done

if [ -z "${CHOSEN_IMAGE}" ]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    CHAIN_REGISTRY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
    echo -e "${YELLOW}Local validator Docker image not found. Building from ${CHAIN_REGISTRY_DIR}/Dockerfile.local...${NC}"
    docker build -f "${CHAIN_REGISTRY_DIR}/Dockerfile.local" -t chain-registry-node-secure:latest "${CHAIN_REGISTRY_DIR}"
    CHOSEN_IMAGE="chain-registry-node-secure:latest"
fi

echo -e "\n${YELLOW}Starting independent validator container using ${CHOSEN_IMAGE}...${NC}"

# Remove old container if it exists
docker rm -f "creg-validator-${NODE_ID}" >/dev/null 2>&1 || true

docker run -d \
    --name "creg-validator-${NODE_ID}" \
    --restart unless-stopped \
    --env-file "${ENV_FILE}" \
    -p "${HTTP_PORT}:8080" \
    -p "${P2P_PORT}:9000" \
    -v "${DATA_DIR}/chain-data:/data" \
    "${CHOSEN_IMAGE}"

echo -e "\n${GREEN}============================================================${NC}"
echo -e "${GREEN}   Validator Node Successfully Launched!                    ${NC}"
echo -e "${GREEN}============================================================${NC}"
echo -e "Node ID:         ${CYAN}${NODE_ID}${NC}"
echo -e "Data Directory:  ${CYAN}${DATA_DIR}${NC}"
echo -e "Key Location:    ${CYAN}${DATA_DIR}/validator.key${NC}"
echo -e "Docker Image:    ${CYAN}${CHOSEN_IMAGE}${NC}"
echo -e "P2P Port:        ${CYAN}${P2P_PORT} (Firewall: OPEN TCP/UDP ${P2P_PORT})${NC}"
echo -e "HTTP RPC Port:   ${CYAN}${HTTP_PORT} (Keep private or proxy with TLS)${NC}"
echo -e "Bootnode Peer:   ${CYAN}${BOOTNODE}${NC}"
echo -e ""
echo -e "${YELLOW}--- KEY BACKUP NOTICE ---${NC}"
echo -e "Validator Key:   ${YELLOW}${VALIDATOR_KEY}${NC}"
echo -e "Save this key string securely! To restore this node anytime, pass:"
echo -e "  CUSTOM_KEY=\"${VALIDATOR_KEY}\" ./testnet/start-external-validator.sh"
echo -e ""
echo -e "${YELLOW}--- REMOTE VERIFICATION ---${NC}"
echo -e "Local query:     curl http://localhost:${HTTP_PORT}/v1/health"
echo -e "Remote query:    curl http://<YOUR_SERVER_IP>:${HTTP_PORT}/v1/health"
echo -e "Global Explorer: https://explorer.testnet.cregnet.dev (Validators Tab)"
echo -e ""
echo -e "${YELLOW}--- STAKING FOR REWARDS ---${NC}"
echo -e "1. Drip testnet tokens:  ${CYAN}creg testnet drip${NC}"
echo -e "2. Deposit stake (100):  ${CYAN}creg stake 100${NC}"
echo -e "3. Monitor live logs:    ${CYAN}docker logs -f creg-validator-${NODE_ID}${NC}\n"
