#!/bin/bash
# Validator Key Generation Script for Chain Registry
#
# Generates the validator key for Chain Registry testnet.

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
KEYS_DIR="$PROJECT_ROOT/validator-keys"
ENV_FILE="$PROJECT_ROOT/.env"

# Number of validators to generate
NUM_VALIDATORS=${1:-1}

print_header() {
    echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║       Chain Registry - Validator Key Generator             ║${NC}"
    echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
    echo ""
}

print_architecture_note() {
    echo -e "${YELLOW}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${YELLOW}║  ARCHITECTURE NOTE:                                        ║${NC}"
    echo -e "${YELLOW}║                                                            ║${NC}"
    echo -e "${YELLOW}║  PRODUCTION: One validator per PC ONLY                     ║${NC}"
    echo -e "${YELLOW}║  TESTING: Multiple validators on one PC is OK              ║${NC}"
    echo -e "${YELLOW}║                                                            ║${NC}"
    echo -e "${YELLOW}║  This script is for TESTNET TESTING ONLY                   ║${NC}"
    echo -e "${YELLOW}╚════════════════════════════════════════════════════════════╝${NC}"
    echo ""
}

check_dependencies() {
    echo -e "${BLUE}[INFO]${NC} Checking dependencies..."
    
    # Check if cargo is available
    if ! command -v cargo &> /dev/null; then
        echo -e "${RED}[ERROR]${NC} Rust/Cargo not found. Please install Rust first."
        echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        exit 1
    fi
    
    echo -e "${GREEN}[OK]${NC} Dependencies checked"
}

generate_keys() {
    echo -e "${BLUE}[INFO]${NC} Generating $NUM_VALIDATORS validator key(s)..."
    
    # Create keys directory
    mkdir -p "$KEYS_DIR"
    
    # Backup existing .env if present
    if [ -f "$ENV_FILE" ]; then
        cp "$ENV_FILE" "$ENV_FILE.backup.$(date +%Y%m%d_%H%M%S)"
        echo -e "${YELLOW}[WARN]${NC} Backed up existing .env file"
    fi
    
    # Create new .env from example if doesn't exist
    if [ ! -f "$ENV_FILE" ]; then
        if [ -f "$PROJECT_ROOT/.env.example" ]; then
            cp "$PROJECT_ROOT/.env.example" "$ENV_FILE"
        else
            touch "$ENV_FILE"
        fi
    fi
    
    # Generate keys for each validator
    for i in $(seq 1 $NUM_VALIDATORS); do
        echo ""
        echo -e "${BLUE}[INFO]${NC} Generating Validator $i key..."
        
        # Generate key using creg CLI
        KEY_OUTPUT=$(cd "$PROJECT_ROOT" && cargo run --quiet --bin creg -- keygen --ed2559 2>/dev/null || echo "")
        
        if [ -z "$KEY_OUTPUT" ]; then
            # Fallback: generate with openssl
            PRIVATE_KEY=$(openssl rand -hex 32)
            PUBLIC_KEY=$(echo -n "$PRIVATE_KEY" | openssl dgst -sha256 -binary | openssl enc -base64)
        else
            # Parse output (assuming format: "Private: <key>\nPublic: <key>")
            PRIVATE_KEY=$(echo "$KEY_OUTPUT" | grep -i "private" | awk '{print $2}')
            PUBLIC_KEY=$(echo "$KEY_OUTPUT" | grep -i "public" | awk '{print $2}')
        fi
        
        # Save to individual file
        cat > "$KEYS_DIR/validator-$i.env" << EOF
# Validator $i Configuration
# Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)

NODE${i}_VALIDATOR_KEY=$PRIVATE_KEY
NODE${i}_PUBLIC_KEY=$PUBLIC_KEY
NODE${i}_ID=node-$i
NODE${i}_DATA_DIR=./data/node-$i
EOF
        
        # Update main .env file
        if grep -q "NODE${i}_VALIDATOR_KEY=" "$ENV_FILE"; then
            # Update existing
            sed -i "s/NODE${i}_VALIDATOR_KEY=.*/NODE${i}_VALIDATOR_KEY=$PRIVATE_KEY/" "$ENV_FILE"
        else
            # Add new
            echo "" >> "$ENV_FILE"
            echo "# Validator $i" >> "$ENV_FILE"
            echo "NODE${i}_VALIDATOR_KEY=$PRIVATE_KEY" >> "$ENV_FILE"
        fi
        
        echo -e "${GREEN}[OK]${NC} Validator $i: Key generated and saved"
        echo -e "  Private: ${YELLOW}${PRIVATE_KEY:0:16}...${NC}"
        echo -e "  Config:  $KEYS_DIR/validator-$i.env"
    done
    
    echo ""
    echo -e "${GREEN}[SUCCESS]${NC} Generated $NUM_VALIDATORS validator key(s)"
}

create_validator_configs() {
    echo ""
    echo -e "${BLUE}[INFO]${NC} Creating validator configuration files..."
    
    for i in $(seq 1 $NUM_VALIDATORS); do
        CONFIG_FILE="$KEYS_DIR/validator-$i-docker-compose.yml"
        
        # Calculate API port (8080 + i - 1)
        API_PORT=$((8080 + i - 1))
        # Calculate P2P port (9000 + i - 1)
        P2P_PORT=$((9000 + i - 1))
        # Calculate gRPC port (50051 + i - 1)
        GRPC_PORT=$((50051 + i - 1))
        
        cat > "$CONFIG_FILE" << EOF
# Validator $i Docker Compose
# Generated automatically - DO NOT EDIT MANUALLY

version: "3.9"

services:
  node-$i:
    build:
      context: ..
      dockerfile: Dockerfile.minimal
    container_name: creg-validator-$i
    environment:
      CREG_NODE_ID: "node-$i"
      CREG_IS_VALIDATOR: "true"
      CREG_VALIDATOR_KEY: "\${NODE${i}_VALIDATOR_KEY}"
      CREG_LISTEN: "0.0.0.0:$API_PORT"
      CREG_API_PORT: "$API_PORT"
      CREG_GRPC_PORT: "$GRPC_PORT"
      CREG_P2P_LISTEN: "/ip4/0.0.0.0/tcp/$P2P_PORT"
      CREG_DATA_DIR: "/data"
      CREG_SINGLE_VALIDATOR_MODE: "false"
      CREG_DEV_SANDBOX: "true"
      CREG_ETH_RPC: "http://anvil:8545"
      RUST_LOG: "info,chain_registry_node=debug"
    ports:
      - "$API_PORT:$API_PORT"
      - "$GRPC_PORT:$GRPC_PORT"
      - "$P2P_PORT:$P2P_PORT"
    volumes:
      - ../data/node-$i:/data
    networks:
      - creg-network
    depends_on:
      - anvil
      - ipfs
    restart: unless-stopped

networks:
  creg-network:
    external: true
    name: chain-registry_creg-network
EOF
        
        echo -e "${GREEN}[OK]${NC} Validator $i: Config created (API: $API_PORT, P2P: $P2P_PORT)"
    done
}

print_summary() {
    echo ""
    echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║                    GENERATION COMPLETE                     ║${NC}"
    echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "Files created:"
    echo -e "  ${GREEN}•${NC} $ENV_FILE (updated with validator keys)"
    echo -e "  ${GREEN}•${NC} $KEYS_DIR/validator-{1..$NUM_VALIDATORS}.env"
    echo -e "  ${GREEN}•${NC} $KEYS_DIR/validator-{1..$NUM_VALIDATORS}-docker-compose.yml"
    echo ""
    echo -e "Next steps:"
    echo -e "  1. Review ${YELLOW}.env${NC} file"
    echo -e "  2. Start infrastructure: ${YELLOW}docker-compose up -d anvil ipfs${NC}"
    echo -e "  3. Start validator 1:   ${YELLOW}docker-compose up -d node${NC}"
    if [ $NUM_VALIDATORS -gt 1 ]; then
        echo -e "  4. Start other validators using their compose files"
    fi
    echo ""
    echo -e "${YELLOW}REMINDER:${NC} This multi-validator setup is for TESTING ONLY"
    echo -e "In production, run ONE validator per PC."
    echo ""
}

# Main
print_header
print_architecture_note
check_dependencies
generate_keys
create_validator_configs
print_summary
