#!/usr/bin/env bash
# ==============================================================================
# Chain Registry (CREG) — Independent External Validator Launch Script
# ==============================================================================
# Launch an external validator for the coordinated R7 testnet. Voting requires
# on-chain admission through the validator onboarding checklist.
# ==============================================================================

set -euo pipefail
umask 077

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

NODE_ID="${NODE_ID:-ext-val-primary}"
ETH_RPC="${ETH_RPC:-${CREG_ETH_RPC:-https://sepolia.gateway.tenderly.co}}"
HTTP_PORT="${HTTP_PORT:-8080}"
P2P_PORT="${P2P_PORT:-9000}"
CUSTOM_KEY="${CUSTOM_KEY:-}"
CHAIN_ID="creg-phase-c-testnet-2026-09-26-r7"
SPEC_URL="https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json"
CONTAINER_NAME="creg-validator-$NODE_ID"
CREG_ONBOARDING_KEY_FILE="${CREG_ONBOARDING_KEY_FILE:-}"
HTTP_BIND="$HTTP_PORT"
if [ -n "$CREG_ONBOARDING_KEY_FILE" ]; then
    EXISTING_CONTAINERS=$(docker container ls -a --format '{{.Names}}')
    EXPECTED_LABEL="$CHAIN_ID/$CREG_ONBOARDING_PUBLIC_KEY"
    if printf '%s\n' "$EXISTING_CONTAINERS" | grep -Fxq "$CONTAINER_NAME"; then
        MATCHING_CONTAINERS=$(docker container ls -a --filter "label=dev.creg.validator=$EXPECTED_LABEL" --format '{{.Names}}')
        if ! printf '%s\n' "$MATCHING_CONTAINERS" | grep -Fxq "$CONTAINER_NAME"; then
            echo "Container $CONTAINER_NAME belongs to another configuration. Review it before starting this validator." >&2
            exit 1
        fi
        docker start "$CONTAINER_NAME"
        exit 0
    fi
    HTTP_BIND="127.0.0.1:"
    if [ "$HTTP_PORT" != 0 ]; then
        HTTP_BIND="127.0.0.1:$HTTP_PORT"
    fi

    # The enrollment key stays on the host while the observer syncs R7.
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    PROFILE="$SCRIPT_DIR/phase-c/r7/observer.docker-env"
    REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
    SCANNER_DIR="$REPO_ROOT/docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r14-ansi-regex-bundle"
    if [ ! -f "$PROFILE" ] || [ ! -f "$SCANNER_DIR/rules/supply_chain.yar" ] ||
        [ ! -f "$SCANNER_DIR/known_malicious_hashes.json" ] || [ ! -f "$SCANNER_DIR/osv_snapshot.json" ]; then
        echo "The reviewed R7 observer profile or scanner bundle is missing from the CREG checkout." >&2
        exit 1
    fi
    REVIEWED_IMAGE_ID='sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
    REVIEWED_CONFIG_ID='sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9'
    if docker image inspect "$REVIEWED_IMAGE_ID" >/dev/null 2>&1; then
        IMAGE="$REVIEWED_IMAGE_ID"
    elif docker image inspect "$REVIEWED_CONFIG_ID" >/dev/null 2>&1; then
        IMAGE="$REVIEWED_CONFIG_ID"
    else
        IMAGE="us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@$REVIEWED_IMAGE_ID"
        if ! docker pull "$IMAGE"; then
            echo "The reviewed R7 image is unavailable. Use the maintainer-provided image." >&2
            exit 1
        fi
    fi
    OBSERVER_DATA="$HOME/.creg-validator-$NODE_ID/onboarding-observer-r7"
    if [ "$NODE_ID" = "ext-val-primary" ]; then
        OBSERVER_DATA="$HOME/.creg-validator/onboarding-observer-r7"
    fi
    mkdir -p "$OBSERVER_DATA"
    if [ -n "${CREG_OBSERVER_IMAGE:-}" ]; then
        IMAGE="$CREG_OBSERVER_IMAGE"
    fi
    EXTRA_AUTO_ARGS=()
    if [ "${CREG_ADMISSION_AUTO_ACTIVATE:-false}" = "true" ] || [ "${CREG_ADMISSION_AUTO_ACTIVATE:-}" = "1" ]; then
        EXTRA_AUTO_ARGS+=(
            --env "CREG_ADMISSION_AUTO_ACTIVATE=true"
            --env "CREG_VALIDATOR_KEY_FILE=/keys/validator.key"
            --mount "type=bind,source=$CREG_ONBOARDING_KEY_FILE,target=/keys/validator.key,readonly"
        )
        # ADR-017: the ML-DSA-44 seed that `creg validator onboard` keeps next
        # to the Ed25519 key (validator.key -> validator.pq.key).
        PQ_KEY_FILE="${CREG_ONBOARDING_KEY_FILE%.key}.pq.key"
        if [ -f "$PQ_KEY_FILE" ]; then
            EXTRA_AUTO_ARGS+=(
                --env "CREG_VALIDATOR_PQ_KEY_FILE=/keys/validator.pq.key"
                --mount "type=bind,source=$PQ_KEY_FILE,target=/keys/validator.pq.key,readonly"
            )
        fi
    fi
    docker run -d --name "$CONTAINER_NAME" --restart unless-stopped \
        --label "dev.creg.validator=$EXPECTED_LABEL" --env-file "$PROFILE" \
        --env "CREG_NODE_ID=$NODE_ID" --env "CREG_IS_VALIDATOR=false" \
        --publish "$HTTP_BIND:8080" \
        --user "$(id -u):$(id -g)" \
        --mount "type=bind,source=$OBSERVER_DATA,target=/data" \
        --mount "type=bind,source=$SCANNER_DIR,target=/scanner,readonly" \
        "${EXTRA_AUTO_ARGS[@]}" "$IMAGE" >/dev/null
    echo "R7 observer started. The onboarding page will show its sync status."
    exit 0
fi

# Isolated data dir per node ID to allow running multiple nodes on one host
if [ "${NODE_ID}" = "ext-val-primary" ]; then
    DATA_DIR="${HOME}/.creg-validator"
else
    DATA_DIR="${HOME}/.creg-validator-${NODE_ID}"
fi
# Keep R7 state separate from the retired network's database. Pre-create the
# bind mount as the host user so Docker does not create it as root.
CHAIN_DATA_DIR="${DATA_DIR}/chain-data-r7"
mkdir -p "${CHAIN_DATA_DIR}"
ENV_FILE="${DATA_DIR}/validator.env"

# Generate, load, or use custom Validator Key
KEY_PATH="$DATA_DIR/validator.key"
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

# ADR-017: an independent ML-DSA-44 seed next to the Ed25519 key, the same
# file `creg validator onboard` creates and registers. The node signs with it
# only from the hybrid activation height in the signed chain spec.
PQ_KEY_PATH="$DATA_DIR/validator.pq.key"
if [ ! -f "$PQ_KEY_PATH" ]; then
    openssl rand -hex 32 > "$PQ_KEY_PATH"
    chmod 600 "$PQ_KEY_PATH"
    echo -e "${GREEN}[OK] Generated new ML-DSA-44 seed: ${PQ_KEY_PATH}${NC}"
else
    echo -e "${GREEN}[OK] Loaded existing ML-DSA-44 seed from ${PQ_KEY_PATH}${NC}"
fi
VALIDATOR_PQ_KEY=$(tr -d '\r\n ' < "$PQ_KEY_PATH")
if ! printf '%s' "$VALIDATOR_PQ_KEY" | grep -Eq '^(0x)?[0-9a-fA-F]{64}$'; then
    echo -e "${RED}The ML-DSA-44 seed at ${PQ_KEY_PATH} must contain 32 bytes of hex.${NC}" >&2
    exit 1
fi

# Operator API Key
if [ ! -f "${DATA_DIR}/operator_api.key" ]; then
    OPERATOR_KEY=$(openssl rand -hex 32)
    echo "${OPERATOR_KEY}" > "${DATA_DIR}/operator_api.key"
    chmod 600 "${DATA_DIR}/operator_api.key"
else
    OPERATOR_KEY=$(cat "${DATA_DIR}/operator_api.key" | tr -d '\r\n ')
fi

# creg-node verifies the spec against its pinned key and adopts its contracts
# and bootnodes. Only an explicitly configured BOOTNODE overrides the seeds.
cat <<EOF > "${ENV_FILE}"
CREG_TESTNET=true
CREG_NODE_ID=${NODE_ID}
CREG_DATA_DIR=/data
CREG_IS_VALIDATOR=true
CREG_NODE_ROLE=validator
CREG_VALIDATOR_KEY=${VALIDATOR_KEY}
CREG_VALIDATOR_PQ_KEY=${VALIDATOR_PQ_KEY}
CREG_OPERATOR_API_KEY=${OPERATOR_KEY}
CREG_ETH_RPC=${ETH_RPC}
CREG_CHAIN_ID=${CHAIN_ID}
CREG_CHAIN_SPEC_URL=${SPEC_URL}
CREG_SPEC_SIGNATURE_URL=${SPEC_URL}.sig
CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev
CREG_P2P_LISTEN=/ip4/0.0.0.0/tcp/9000
CREG_PEERS=https://api.testnet.cregnet.dev
CREG_LISTEN=0.0.0.0:8080
CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false
RUST_LOG=info,chain_registry_node=debug,consensus=info
EOF
if [ -n "${BOOTNODE:-}" ]; then
    printf 'CREG_P2P_SEEDS=%s\n' "${BOOTNODE}" >> "${ENV_FILE}"
fi
chmod 600 "${ENV_FILE}"

# Auto-detect available Docker image
CHOSEN_IMAGE=""
for candidate in chain-registry-node-secure:latest creg-node:fleet creg-node:latest; do
    if docker image inspect "${candidate}" >/dev/null 2>&1; then
        CHOSEN_IMAGE="${candidate}"
        break
    fi
done

if [ -z "${CHOSEN_IMAGE}" ]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    CHAIN_REGISTRY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
    echo -e "${YELLOW}Local validator Docker image not found. Building from ${CHAIN_REGISTRY_DIR}/Dockerfile (multi-stage, no host toolchain needed; takes a while on first run)...${NC}"
    docker build -f "${CHAIN_REGISTRY_DIR}/Dockerfile" -t chain-registry-node-secure:latest "${CHAIN_REGISTRY_DIR}"
    CHOSEN_IMAGE="chain-registry-node-secure:latest"
fi

echo -e "\n${YELLOW}Starting independent validator container using ${CHOSEN_IMAGE}...${NC}"

docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true

docker run -d \
    --name "$CONTAINER_NAME" \
    --restart unless-stopped \
    --user "$(id -u):$(id -g)" \
    --env-file "${ENV_FILE}" \
    -p "$HTTP_BIND:8080" \
    -p "${P2P_PORT}:9000" \
    -v "${CHAIN_DATA_DIR}:/data" \
    "${CHOSEN_IMAGE}"

echo -e "\n${GREEN}============================================================${NC}"
echo -e "${GREEN}   R7 Validator Container Started                          ${NC}"
echo -e "${GREEN}============================================================${NC}"
echo -e "Node ID:         ${CYAN}${NODE_ID}${NC}"
echo -e "Chain Data:      ${CYAN}${CHAIN_DATA_DIR}${NC}"
echo -e "Key Location:    ${CYAN}$KEY_PATH${NC}"
echo -e "ML-DSA-44 Seed:  ${CYAN}$PQ_KEY_PATH${NC}"
echo -e "Docker Image:    ${CYAN}${CHOSEN_IMAGE}${NC}"
echo -e "P2P Port:        ${CYAN}${P2P_PORT} (Firewall: OPEN TCP/UDP ${P2P_PORT})${NC}"
if [ "$HTTP_PORT" = 0 ]; then
    echo -e "HTTP RPC:        Docker-assigned loopback port"
else
    echo -e "HTTP RPC Port:   ${CYAN}${HTTP_PORT} (Keep private or proxy with TLS)${NC}"
fi
echo -e "P2P Seeds:       ${CYAN}${BOOTNODE:-from the signed R7 spec}${NC}"
echo -e ""
echo -e "${YELLOW}--- KEY BACKUP NOTICE ---${NC}"
echo -e "Back up ${DATA_DIR}/validator.key and ${PQ_KEY_PATH} through the operator's secret storage."
echo -e "Do not print or paste the private key into terminal history."
echo -e ""
echo -e "${YELLOW}--- REMOTE VERIFICATION ---${NC}"
if [ "$HTTP_PORT" != 0 ]; then
    echo -e "Local query:     curl http://localhost:${HTTP_PORT}/v1/health"
    echo -e "Remote query:    curl http://<YOUR_SERVER_IP>:${HTTP_PORT}/v1/health"
fi
echo -e "Global Explorer: https://explorer.testnet.cregnet.dev (Validators Tab)"
echo -e ""
echo -e "${YELLOW}--- R7 VALIDATOR ADMISSION ---${NC}"
echo -e "Follow docs/VALIDATOR_ONBOARDING_CHECKLIST.md for staking, registration, and approval."
echo -e "Monitor live logs: ${CYAN}docker logs -f creg-validator-${NODE_ID}${NC}\n"
