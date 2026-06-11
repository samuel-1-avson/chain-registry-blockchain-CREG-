#!/usr/bin/env bash
# Build/start hub on edge VM and recreate Caddy for CREG_PUBLIC_JOIN_HOST.
set -euo pipefail

REPO_ROOT="${REPO_ROOT:-$HOME/creg-hosting/chain-registry-blockchain-CREG-/chain-registry}"
cd "$REPO_ROOT"

pkill -f 'testnet/start-cloud-edge-gcp.sh' 2>/dev/null || true
sed -i 's/\r$//' testnet/sepolia-3node.env
cp -f testnet/caddy/hub-edge.caddy.example testnet/caddy/hub.caddy

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  DOCKER=(sudo docker)
fi

# shellcheck source=testnet/_source-sepolia-env.sh
source testnet/_source-sepolia-env.sh
creg_source_sepolia_env testnet/sepolia-3node.env

sed "s/@VALIDATOR_IP@/${CREG_VALIDATOR_VM_INTERNAL_IP}/g" \
  testnet/nginx/explorer-fleet.conf.template > testnet/nginx/explorer-fleet.conf

export COMPOSE_PROFILES=hub
export CREG_CLOUD_CADDYFILE=./caddy/Caddyfile.fleet
export CREG_EXPLORER_NGINX_CONF=./nginx/explorer-fleet.conf

COMPOSE=(
  "${DOCKER[@]}" compose
  -f testnet/docker-compose.cloud-edge.yml
  -f testnet/docker-compose.cloud-edge-ingress.yml
  --env-file testnet/sepolia-3node.env
)

echo "=== Building hub-api + hub-web ==="
env CREG_CLOUD_CADDYFILE="$CREG_CLOUD_CADDYFILE" CREG_EXPLORER_NGINX_CONF="$CREG_EXPLORER_NGINX_CONF" \
  "${COMPOSE[@]}" --profile hub up -d --build hub-api hub-web

echo "=== Recreating Caddy (TLS for ${CREG_PUBLIC_JOIN_HOST:-join host}) ==="
env CREG_CLOUD_CADDYFILE="$CREG_CLOUD_CADDYFILE" CREG_EXPLORER_NGINX_CONF="$CREG_EXPLORER_NGINX_CONF" \
  "${COMPOSE[@]}" up -d --force-recreate caddy

sleep 8
curl -s -o /dev/null -w 'local8094:%{http_code}\n' http://127.0.0.1:8094/ || true
curl -s http://127.0.0.1:8095/api/health || true
curl -sk -o /dev/null -w 'https_join:%{http_code}\n' "https://${CREG_PUBLIC_JOIN_HOST}/" || true
echo "=== docker ps (hub) ==="
"${DOCKER[@]}" ps --format 'table {{.Names}}\t{{.Status}}' | grep -E 'hub|caddy' || true
