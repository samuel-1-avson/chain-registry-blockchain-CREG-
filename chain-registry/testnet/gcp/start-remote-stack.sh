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
chmod +x testnet/start-3node-gcp.sh 2>/dev/null || true
wait_for_docker
bash testnet/start-3node-gcp.sh
echo ""
echo "=== Recent Caddy logs ==="
docker logs --tail 40 creg-3node-caddy 2>/dev/null || true
