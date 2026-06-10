#!/usr/bin/env bash
# Run on VM after sync-local-repo + push-env.
set -euo pipefail

GITHUB_REPO="${GITHUB_REPO:-samuel-1-avson/chain-registry-blockchain-CREG-}"
WORKDIR="${WORKDIR:-$HOME/creg-hosting}"
REPO_SLUG="${GITHUB_REPO##*/}"
REPO_ROOT="$WORKDIR/$REPO_SLUG/chain-registry"

wait_for_docker() {
  for i in $(seq 1 120); do
    if docker info >/dev/null 2>&1 || sudo docker info >/dev/null 2>&1; then
      echo "Docker ready (attempt $i)"
      return 0
    fi
    sleep 10
  done
  echo "Docker not ready after 20 minutes" >&2
  exit 1
}

cd "$REPO_ROOT"
if [[ ! -f testnet/sepolia-3node.env ]]; then
  echo "Missing testnet/sepolia-3node.env - run push-env.ps1 from workstation" >&2
  exit 1
fi
if [[ ! -f testnet/start-3node-gcp.sh ]]; then
  echo "Missing testnet/start-3node-gcp.sh - run sync-local-repo.ps1 from workstation" >&2
  exit 1
fi
chmod 600 testnet/sepolia-3node.env
chmod +x testnet/_source-sepolia-env.sh testnet/start-3node-gcp.sh testnet/start-cloud-edge-gcp.sh testnet/start-validator-fleet-gcp.sh 2>/dev/null || true
# Strip UTF-8 BOM in place if push-env ran before normalization (Linux source breaks on BOM).
if head -c 3 testnet/sepolia-3node.env | grep -q $'^\xEF\xBB\xBF'; then
  echo "Stripping UTF-8 BOM from testnet/sepolia-3node.env ..."
  tail -c +4 testnet/sepolia-3node.env > testnet/sepolia-3node.env.nobom
  mv testnet/sepolia-3node.env.nobom testnet/sepolia-3node.env
  chmod 600 testnet/sepolia-3node.env
fi
wait_for_docker
CADDY_CONTAINER=creg-3node-caddy
if grep -qE '^\s*CREG_VALIDATOR_FLEET_MODE\s*=\s*true' testnet/sepolia-3node.env 2>/dev/null; then
  echo "CREG_VALIDATOR_FLEET_MODE=true — cloud edge only (validators on creg-validator-vm)"
  CADDY_CONTAINER=creg-cloud-caddy
  bash testnet/start-cloud-edge-gcp.sh
elif grep -qE '^\s*CREG_HYBRID_MODE\s*=\s*true' testnet/sepolia-3node.env 2>/dev/null; then
  echo "CREG_HYBRID_MODE=true — cloud edge only (validators on operator PC)"
  CADDY_CONTAINER=creg-cloud-caddy
  bash testnet/start-cloud-edge-gcp.sh
else
  echo "Legacy mode — full 3-node stack on edge VM"
  bash testnet/start-3node-gcp.sh
fi
echo ""
echo "=== Recent Caddy logs ($CADDY_CONTAINER) ==="
docker logs --tail 40 "$CADDY_CONTAINER" 2>/dev/null || sudo docker logs --tail 40 "$CADDY_CONTAINER" 2>/dev/null || true
