#!/usr/bin/env bash
set -euo pipefail
REPO="${REPO:-$HOME/creg-hosting/chain-registry-blockchain-CREG-/chain-registry}"
cd "$REPO/testnet"
cp /tmp/sepolia-public-rpc.caddy caddy/
cp /tmp/creg-node-jsonrpc.caddy caddy/ 2>/dev/null || true
cp /tmp/Caddyfile.fleet caddy/ 2>/dev/null || true
cp /tmp/Caddyfile.hybrid caddy/ 2>/dev/null || true
cp /tmp/explorer-fleet.conf.template nginx/
# shellcheck source=testnet/_source-sepolia-env.sh
source ./_source-sepolia-env.sh
creg_source_sepolia_env sepolia-3node.env
observer_host="${CREG_OBSERVER_POOL_LB_IP:-}"
observer_port="${CREG_3NODE_NODE3_API_PORT:-28182}"
export CREG_OBSERVER_API_UPSTREAM="${CREG_OBSERVER_API_UPSTREAM:-${observer_host:+${observer_host}:${observer_port}}}"
CREG_OBSERVER_API_UPSTREAM="${CREG_OBSERVER_API_UPSTREAM#http://}"
CREG_OBSERVER_API_UPSTREAM="${CREG_OBSERVER_API_UPSTREAM#https://}"
export CREG_OBSERVER_API_UPSTREAM
if [[ -z "${CREG_OBSERVER_API_UPSTREAM:-}" ]]; then
  echo "Set CREG_OBSERVER_API_UPSTREAM or CREG_OBSERVER_POOL_LB_IP; validators are external to GCP." >&2
  exit 1
fi
sed -e "s/@OBSERVER_UPSTREAM@/${CREG_OBSERVER_API_UPSTREAM}/g" \
  nginx/explorer-fleet.conf.template > nginx/explorer-fleet.conf
export CREG_EXPLORER_NGINX_CONF="./nginx/explorer-fleet.conf"
export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.fleet"
if [[ -n "${CREG_PUBLIC_WAITLIST_HOST:-}" ]]; then
  cp caddy/waitlist-edge.caddy.example caddy/waitlist.caddy
fi
if [[ -n "${CREG_PUBLIC_JOIN_HOST:-}" ]]; then
  cp caddy/hub-edge.caddy.example caddy/hub.caddy
fi
sudo -E docker compose -f docker-compose.cloud-edge.yml -f docker-compose.cloud-edge-ingress.yml \
  --env-file sepolia-3node.env up -d --force-recreate caddy web-explorer
echo "RPC browser UX applied."
