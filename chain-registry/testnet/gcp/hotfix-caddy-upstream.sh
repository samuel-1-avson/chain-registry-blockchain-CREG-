#!/usr/bin/env bash
set -euo pipefail
REPO="${HOME}/creg-hosting/chain-registry-blockchain-CREG-/chain-registry"
cd "${REPO}/testnet"

if [[ -f /tmp/docker-compose.cloud-edge-ingress.yml ]]; then
  cp /tmp/docker-compose.cloud-edge-ingress.yml ./
  tr -d '\r' < ./docker-compose.cloud-edge-ingress.yml > ./docker-compose.cloud-edge-ingress.yml.lf
  mv ./docker-compose.cloud-edge-ingress.yml.lf ./docker-compose.cloud-edge-ingress.yml
fi

# Recreate Caddy with observer upstream (uses repo caddy/Caddyfile.fleet + creg-node-jsonrpc.caddy).
# shellcheck source=testnet/_source-sepolia-env.sh
source ./_source-sepolia-env.sh
creg_source_sepolia_env "./sepolia-3node.env"

export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.fleet"
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
if [[ -n "${CREG_PUBLIC_WAITLIST_HOST:-}" ]]; then
  cp caddy/waitlist-edge.caddy.example caddy/waitlist.caddy
fi
if [[ -n "${CREG_PUBLIC_JOIN_HOST:-}" ]]; then
  cp caddy/hub-edge.caddy.example caddy/hub.caddy
fi
echo "Recreating caddy + explorer — API upstream ${CREG_OBSERVER_API_UPSTREAM}"

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  DOCKER=(sudo -E docker)
fi

COMPOSE=("${DOCKER[@]}" compose -f docker-compose.cloud-edge.yml -f docker-compose.cloud-edge-ingress.yml --env-file sepolia-3node.env)
export CREG_CLOUD_CADDYFILE CREG_OBSERVER_API_UPSTREAM CREG_EXPLORER_NGINX_CONF
"${COMPOSE[@]}" up -d --force-recreate caddy web-explorer

"${DOCKER[@]}" exec creg-cloud-caddy env | grep CREG_OBSERVER_API_UPSTREAM || true
curl -fsS "http://127.0.0.1:443/v1/health" -k -H "Host: api.testnet.cregnet.dev" || true
