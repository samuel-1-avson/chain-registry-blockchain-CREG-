#!/usr/bin/env bash
# Build creg-node from synced source and export for observer VM import.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/testnet/sepolia-3node.env"
LOG="${1:-/tmp/build-export.log}"

exec >"$LOG" 2>&1

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  DOCKER=(sudo docker)
fi

cd "$REPO_ROOT"
COMPOSE=("${DOCKER[@]}" compose -f testnet/docker-compose.validator-fleet.yml --env-file "$ENV_FILE")

echo "=== Building creg-node-1 ==="
"${COMPOSE[@]}" build creg-node-1
# Compose names the image CREG_FLEET_IMAGE (ghcr.io/.../latest), not creg-node:fleet.
fleet_ref="${CREG_FLEET_IMAGE:-ghcr.io/chain-registry/chain-registry:latest}"
if ! "${DOCKER[@]}" image inspect "$fleet_ref" >/dev/null 2>&1; then
  built_id="$("${COMPOSE[@]}" images -q creg-node-1 2>/dev/null | head -n1)"
  if [[ -z "$built_id" ]]; then
    echo "ERROR: no image from compose build (tried $fleet_ref and creg-node-1)" >&2
    exit 1
  fi
  fleet_ref="$built_id"
fi

"${DOCKER[@]}" tag "$fleet_ref" creg-node:fleet
echo "=== Tagged creg-node:fleet -> $fleet_ref ==="
echo "=== Exporting creg-node:fleet to /tmp/creg-node-fleet.tgz ==="
"${DOCKER[@]}" save creg-node:fleet | gzip -1 > /tmp/creg-node-fleet.tgz
echo export_done > /tmp/observer-image-export.done
echo "=== Done ==="
