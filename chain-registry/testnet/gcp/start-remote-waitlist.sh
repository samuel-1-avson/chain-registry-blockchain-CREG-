#!/usr/bin/env bash
# Run on VM after sync-local-repo + waitlist dist present.
set -euo pipefail

REPO_ROOT="${1:-$HOME/creg-hosting/chain-registry-blockchain-CREG-/chain-registry}"
cd "$REPO_ROOT"

if [[ ! -f testnet/waitlist/dist/index.html ]]; then
  echo "Missing testnet/waitlist/dist/index.html — run deploy-waitlist.ps1 from workstation" >&2
  exit 1
fi

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  DOCKER=(sudo docker)
fi

COMPOSE=("${DOCKER[@]}" compose
  -f testnet/docker-compose.3node.yml
  -f testnet/docker-compose.3node-services.yml
  -f testnet/docker-compose.waitlist.yml
  -f testnet/docker-compose.3node-ingress.yml
  --env-file testnet/sepolia-3node.env
)

echo "=== Building waitlist image ==="
"${COMPOSE[@]}" build waitlist
echo "=== Starting waitlist ==="
"${COMPOSE[@]}" up -d waitlist
echo "=== Recreating Caddy (new site block + cert) ==="
"${COMPOSE[@]}" up -d --force-recreate caddy
sleep 5
"${COMPOSE[@]}" ps waitlist caddy
docker logs --tail 30 creg-3node-caddy 2>/dev/null || true
