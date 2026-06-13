#!/usr/bin/env bash
set -euo pipefail
REPO="${REPO:-$HOME/creg-hosting/chain-registry-blockchain-CREG-/chain-registry}"
cd "$REPO/testnet"
cp /tmp/sepolia-public-rpc.caddy caddy/
cp /tmp/Caddyfile.fleet caddy/ 2>/dev/null || true
cp /tmp/Caddyfile.hybrid caddy/ 2>/dev/null || true
cp /tmp/explorer-fleet.conf.template nginx/
# shellcheck source=testnet/_source-sepolia-env.sh
source ./_source-sepolia-env.sh
creg_source_sepolia_env sepolia-3node.env
observer_host="${CREG_OBSERVER_POOL_LB_IP:-${CREG_VALIDATOR_VM_INTERNAL_IP}}"
observer_port="${CREG_3NODE_NODE3_API_PORT:-28182}"
export CREG_OBSERVER_API_UPSTREAM="${observer_host}:${observer_port}"
sed -e "s/@VALIDATOR_IP@/${CREG_VALIDATOR_VM_INTERNAL_IP}/g" \
  -e "s/@OBSERVER_UPSTREAM@/${CREG_OBSERVER_API_UPSTREAM}/g" \
  nginx/explorer-fleet.conf.template > nginx/explorer-fleet.conf
export CREG_EXPLORER_NGINX_CONF="./nginx/explorer-fleet.conf"
export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.fleet"
sudo -E docker compose -f docker-compose.cloud-edge.yml -f docker-compose.cloud-edge-ingress.yml \
  --env-file sepolia-3node.env up -d --force-recreate caddy web-explorer
echo "RPC browser UX applied."
