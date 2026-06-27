#!/usr/bin/env bash
# Trim edge VM to waitlist + Caddy only (stop testnet containers, keep TLS for waitlist.cregnet.dev).
#
# Run on creg-testnet-vm after waitlist dist is present:
#   ./testnet/gcp/start-waitlist-edge-only.sh
#
# Optional: pass repo root (default: parent of testnet/).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
cd "$REPO_ROOT/testnet"

if [[ ! -f waitlist/dist/index.html ]]; then
  echo "Missing testnet/waitlist/dist/index.html — run deploy-waitlist.ps1 from workstation" >&2
  exit 1
fi

sed -i 's/\r$//' ./sepolia-3node.env 2>/dev/null || true
find gcp -name '*.sh' -exec sed -i 's/\r$//' {} + 2>/dev/null || true

# shellcheck source=testnet/_source-sepolia-env.sh
source ./_source-sepolia-env.sh
creg_source_sepolia_env "./sepolia-3node.env"

if [[ -z "${CREG_PUBLIC_WAITLIST_HOST:-}" ]]; then
  echo "Set CREG_PUBLIC_WAITLIST_HOST in sepolia-3node.env (e.g. waitlist.cregnet.dev)" >&2
  exit 1
fi

export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.waitlist-only"

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  DOCKER=(sudo -E docker)
fi

LEGACY_3NODE=(
  creg-3node-caddy creg-3node-node1 creg-3node-node2 creg-3node-node3
  creg-3node-explorer creg-3node-spec-server creg-3node-ipfs creg-waitlist
)
TRIM_CLOUD=(
  creg-cloud-explorer creg-cloud-faucet creg-cloud-ipfs creg-cloud-spec-server
  creg-cloud-ipfs-perms creg-cloud-hub-web creg-cloud-hub-api
  creg-gcp-prometheus creg-gcp-alertmanager
)

echo "=== Stopping legacy 3-node containers (if any) ==="
for name in "${LEGACY_3NODE[@]}"; do
  "${DOCKER[@]}" rm -f "$name" 2>/dev/null || true
done

echo "=== Stopping non-waitlist cloud-edge containers ==="
for name in "${TRIM_CLOUD[@]}"; do
  "${DOCKER[@]}" stop "$name" 2>/dev/null || true
  "${DOCKER[@]}" rm -f "$name" 2>/dev/null || true
done

COMPOSE=("${DOCKER[@]}" compose
  -f docker-compose.waitlist-edge.yml
  --env-file sepolia-3node.env
)

echo "=== Building waitlist image (if needed) ==="
"${COMPOSE[@]}" build waitlist

echo "=== Starting waitlist-only stack (waitlist + Caddy) ==="
"${DOCKER[@]}" rm -f creg-cloud-caddy 2>/dev/null || true
export CREG_CLOUD_CADDYFILE
env CREG_CLOUD_CADDYFILE="${CREG_CLOUD_CADDYFILE}" "${COMPOSE[@]}" up -d --force-recreate

sleep 3
echo ""
echo "=== Running containers ==="
"${DOCKER[@]}" ps --format 'table {{.Names}}\t{{.Status}}' | grep -E 'waitlist|caddy' || true
echo ""
echo "Verify: curl -fsS https://${CREG_PUBLIC_WAITLIST_HOST}/"
