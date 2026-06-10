#!/usr/bin/env bash
# Start validator fleet on creg-validator-vm (Linux, private VPC).
#
# Usage:
#   ./testnet/start-validator-fleet-gcp.sh

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
  echo "Set CREG_EDGE_INTERNAL_IP in sepolia-3node.env (creg-testnet-vm VPC IP)" >&2
  exit 1
fi

export CREG_IPFS_URL="${CREG_IPFS_URL:-http://${CREG_EDGE_INTERNAL_IP}:${CREG_3NODE_IPFS_HOST_PORT:-15001}}"
export CREG_CHAIN_SPEC_URL="${CREG_CHAIN_SPEC_URL:-http://${CREG_EDGE_INTERNAL_IP}:${CREG_3NODE_SPEC_HOST_PORT:-18888}/chain-spec.json}"

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  if sudo docker info >/dev/null 2>&1; then
    DOCKER=(sudo -E docker)
  else
    echo "Docker not available" >&2
    exit 1
  fi
fi

COMPOSE=("${DOCKER[@]}" compose -f "${SCRIPT_DIR}/docker-compose.validator-fleet.yml" --env-file "$ENV_FILE")
FLEET_IMAGE="${CREG_FLEET_IMAGE:-ghcr.io/chain-registry/chain-registry:latest}"
export CREG_FLEET_IMAGE="$FLEET_IMAGE"

# Default: pull GHCR image (set CREG_FLEET_BUILD=1 to compile on VM via Dockerfile.windows).
use_prebuilt=1
if [[ -n "${CREG_FLEET_BUILD:-}" ]]; then
  use_prebuilt=0
fi

if [[ "$use_prebuilt" -eq 1 ]]; then
  echo "=== Pulling fleet image: ${FLEET_IMAGE} ==="
  if ! "${DOCKER[@]}" pull "$FLEET_IMAGE"; then
    found_local=0
    # Reuse a prior VM build so IAP SSH drops do not force another hour-long compile.
    for local_tag in creg-node:fleet creg-node:latest ghcr.io/chain-registry/chain-registry:latest; do
      if "${DOCKER[@]}" image inspect "$local_tag" >/dev/null 2>&1; then
        echo "Prebuilt pull failed; using local image ${local_tag}" >&2
        export CREG_FLEET_IMAGE="$local_tag"
        FLEET_IMAGE="$local_tag"
        found_local=1
        break
      fi
    done
    if [[ "$found_local" -eq 0 ]]; then
      echo "No local fleet image; falling back to Dockerfile.windows build" >&2
      use_prebuilt=0
    fi
  fi
fi

if [[ "$use_prebuilt" -eq 0 ]]; then
  echo "=== Building ${FLEET_IMAGE} (Dockerfile.windows - libclang for librocksdb-sys) ==="
  "${COMPOSE[@]}" build creg-node-1
  built_id="$("${COMPOSE[@]}" images -q creg-node-1 2>/dev/null | head -n1)"
  if [[ -n "$built_id" ]]; then
    "${DOCKER[@]}" tag "$built_id" creg-node:fleet 2>/dev/null || true
  fi
fi
echo "=== Starting validator fleet (3 nodes) ==="
"${COMPOSE[@]}" down --remove-orphans 2>/dev/null || true
if [[ "$use_prebuilt" -eq 1 ]]; then
  "${COMPOSE[@]}" up -d --pull always --no-build --remove-orphans
else
  "${COMPOSE[@]}" up -d --build --remove-orphans
fi

echo ""
echo "Fleet started. API ports on this host:"
echo "  node1 ${CREG_3NODE_NODE1_API_PORT:-28180}"
echo "  node2 ${CREG_3NODE_NODE2_API_PORT:-28181}"
echo "  node3 ${CREG_3NODE_NODE3_API_PORT:-28182}"
echo "Edge should proxy to CREG_VALIDATOR_VM_INTERNAL_IP with CREG_VALIDATOR_FLEET_MODE=true"
