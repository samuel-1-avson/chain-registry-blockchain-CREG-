#!/usr/bin/env bash
# Start observer read node on creg-observer-pool VM(s).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

ENV_FILE="${SCRIPT_DIR}/sepolia-3node.env"
if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE" >&2
  exit 1
fi

# shellcheck source=testnet/_source-sepolia-env.sh
source "${SCRIPT_DIR}/_source-sepolia-env.sh"
creg_source_sepolia_env "$ENV_FILE"

if [[ -z "${CREG_EDGE_INTERNAL_IP:-}" ]]; then
  echo "Set CREG_EDGE_INTERNAL_IP in sepolia-3node.env" >&2
  exit 1
fi
if [[ -z "${CREG_VALIDATOR_VM_INTERNAL_IP:-}" ]]; then
  echo "Set CREG_VALIDATOR_VM_INTERNAL_IP in sepolia-3node.env" >&2
  exit 1
fi

export CREG_IPFS_URL="${CREG_IPFS_URL:-http://${CREG_EDGE_INTERNAL_IP}:${CREG_3NODE_IPFS_HOST_PORT:-15001}}"
export CREG_CHAIN_SPEC_URL="${CREG_CHAIN_SPEC_URL:-http://${CREG_EDGE_INTERNAL_IP}:${CREG_3NODE_SPEC_HOST_PORT:-18888}/chain-spec.json}"

# Default seeds/peers to validator fleet (override in env for multi-validator layouts).
export CREG_VALIDATOR_P2P_SEEDS="${CREG_VALIDATOR_P2P_SEEDS:-/ip4/${CREG_VALIDATOR_VM_INTERNAL_IP}/tcp/${CREG_3NODE_NODE1_P2P_PORT:-29100},/ip4/${CREG_VALIDATOR_VM_INTERNAL_IP}/tcp/${CREG_3NODE_NODE2_P2P_PORT:-29101}}"
export CREG_VALIDATOR_HTTP_PEERS="${CREG_VALIDATOR_HTTP_PEERS:-http://${CREG_VALIDATOR_VM_INTERNAL_IP}:${CREG_3NODE_NODE1_API_PORT:-28180},http://${CREG_VALIDATOR_VM_INTERNAL_IP}:${CREG_3NODE_NODE2_API_PORT:-28181}}"

if [[ -z "${CREG_OBSERVER_NODE_ID:-}" ]]; then
  host_id="$(hostname -s 2>/dev/null || hostname)"
  export CREG_OBSERVER_NODE_ID="observer-${host_id}"
fi

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  if sudo docker info >/dev/null 2>&1; then
    DOCKER=(sudo -E docker)
  else
    echo "Docker not available" >&2
    exit 1
  fi
fi

FLEET_IMAGE="${CREG_FLEET_IMAGE:-ghcr.io/chain-registry/chain-registry:latest}"
export CREG_FLEET_IMAGE="$FLEET_IMAGE"

echo "=== Pulling observer image: ${FLEET_IMAGE} ==="
"${DOCKER[@]}" pull "$FLEET_IMAGE" || true

COMPOSE=("${DOCKER[@]}" compose -f "${SCRIPT_DIR}/docker-compose.observer-pool.yml" --env-file "$ENV_FILE")
echo "=== Starting observer pool container (${CREG_OBSERVER_NODE_ID}) ==="
"${COMPOSE[@]}" up -d --pull always observer

echo ""
echo "Observer API: http://127.0.0.1:${CREG_OBSERVER_POOL_API_PORT:-28182}/v1/public/health"
echo "Health:       curl -fsS http://127.0.0.1:${CREG_OBSERVER_POOL_API_PORT:-28182}/v1/health"
