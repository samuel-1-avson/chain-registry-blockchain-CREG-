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

# MAL-001: real sandbox (nsjail) is the default for the public fleet.
# Set CREG_FLEET_DEV_SANDBOX=1 only for throwaway dev fleets — it skips the
# secure-image build and lets CREG_DEV_SANDBOX from sepolia-3node.env apply.
SECURE_SANDBOX=1
if [[ "${CREG_FLEET_DEV_SANDBOX:-0}" == "1" ]]; then
  SECURE_SANDBOX=0
  echo "WARNING: CREG_FLEET_DEV_SANDBOX=1 — validators may run WITHOUT a real sandbox. Never use for public profiles." >&2
fi

COMPOSE_FILES=(-f "${SCRIPT_DIR}/docker-compose.validator-fleet.yml")
if [[ "$SECURE_SANDBOX" -eq 1 ]]; then
  COMPOSE_FILES+=(-f "${SCRIPT_DIR}/docker-compose.fleet-sandbox.yml")
fi
COMPOSE=("${DOCKER[@]}" compose "${COMPOSE_FILES[@]}" --env-file "$ENV_FILE")
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
  # Build without fleet-sandbox overlay so images -q resolves the app image,
  # not chain-registry-node-secure:fleet from the MAL-001 compose override.
  COMPOSE_BUILD=("${DOCKER[@]}" compose -f "${SCRIPT_DIR}/docker-compose.validator-fleet.yml" --env-file "$ENV_FILE")
  "${COMPOSE_BUILD[@]}" build creg-node-1
  # Compose tags the build as ${CREG_FLEET_IMAGE} (default ghcr.io/.../latest).
  # Do not use `compose images -q` here — with fleet-sandbox overlays or stale
  # local tags it can return an old creg-node:fleet id instead of the fresh build.
  built_ref="${CREG_FLEET_IMAGE:-ghcr.io/chain-registry/chain-registry:latest}"
  if ! "${DOCKER[@]}" image inspect "$built_ref" >/dev/null 2>&1; then
    built_id="$("${COMPOSE_BUILD[@]}" images -q creg-node-1 2>/dev/null | head -n1)"
    if [[ -z "$built_id" ]]; then
      echo "ERROR: fleet build produced no image (expected ${built_ref})" >&2
      exit 1
    fi
    built_ref="$built_id"
  fi
  "${DOCKER[@]}" tag "$built_ref" creg-node:fleet
  "${DOCKER[@]}" tag "$built_ref" chain-registry-app:latest
  FLEET_IMAGE="creg-node:fleet"
  export CREG_FLEET_IMAGE="$FLEET_IMAGE"
  echo "=== Tagged creg-node:fleet from ${built_ref} ==="
fi
# ── MAL-001: build secure (nsjail) image on top of the resolved fleet image ──
# Dockerfile.secure expects base tag chain-registry-app:latest; retag whatever
# fleet image we resolved (GHCR pull or local build) and compile nsjail on it.
if [[ "$SECURE_SANDBOX" -eq 1 ]]; then
  echo "=== Building secure validator image (nsjail) from ${FLEET_IMAGE} ==="
  "${DOCKER[@]}" tag "$FLEET_IMAGE" chain-registry-app:latest
  "${DOCKER[@]}" build -t chain-registry-node-secure:fleet -f "${REPO_ROOT}/Dockerfile.secure" "$REPO_ROOT"
  "${DOCKER[@]}" run --rm --entrypoint nsjail chain-registry-node-secure:fleet --help >/dev/null
  echo "=== chain-registry-node-secure:fleet ready (nsjail verified) ==="
fi

echo "=== Starting validator fleet (3 nodes) ==="
"${COMPOSE[@]}" down --remove-orphans 2>/dev/null || true
if [[ "$use_prebuilt" -eq 1 && "$SECURE_SANDBOX" -eq 0 ]]; then
  "${COMPOSE[@]}" up -d --pull always --no-build --remove-orphans
else
  # Secure mode: chain-registry-node-secure:fleet is a local-only tag — a
  # registry pull would fail, so start from local images.
  "${COMPOSE[@]}" up -d --no-build --remove-orphans
fi

echo ""
echo "Fleet started. API ports on this host:"
echo "  node1 ${CREG_3NODE_NODE1_API_PORT:-28180}"
echo "  node2 ${CREG_3NODE_NODE2_API_PORT:-28181}"
echo "  node3 ${CREG_3NODE_NODE3_API_PORT:-28182}"
echo "Edge should proxy to CREG_VALIDATOR_VM_INTERNAL_IP with CREG_VALIDATOR_FLEET_MODE=true"
