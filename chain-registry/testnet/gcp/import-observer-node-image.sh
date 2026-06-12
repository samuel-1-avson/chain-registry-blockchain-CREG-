#!/usr/bin/env bash
# Import creg-node:fleet tarball and restart observer pool container.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ENV_FILE="${REPO_ROOT}/testnet/sepolia-3node.env"
TARBALL="${1:-/tmp/creg-node-fleet.tgz}"

if [[ ! -f "$TARBALL" ]]; then
  echo "Missing $TARBALL" >&2
  exit 1
fi

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  DOCKER=(sudo docker)
fi

echo "=== Loading $TARBALL ==="
gunzip -c "$TARBALL" | "${DOCKER[@]}" load
export CREG_FLEET_IMAGE=creg-node:fleet
cd "$REPO_ROOT"
chmod +x testnet/start-observer-pool-gcp.sh
exec ./testnet/start-observer-pool-gcp.sh
