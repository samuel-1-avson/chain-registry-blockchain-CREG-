#!/usr/bin/env bash
set -euo pipefail
REPO="$HOME/creg-hosting/chain-registry-blockchain-CREG-/chain-registry"
SCRIPT_DIR="$REPO/testnet"
cd "$SCRIPT_DIR"
source "${SCRIPT_DIR}/_source-sepolia-env.sh"
creg_source_sepolia_env "${SCRIPT_DIR}/sepolia-3node.env"
export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.fleet"
observer_host="${CREG_OBSERVER_POOL_LB_IP:-${CREG_VALIDATOR_VM_INTERNAL_IP}}"
observer_port="${CREG_3NODE_NODE3_API_PORT:-28182}"
export CREG_OBSERVER_API_UPSTREAM="${observer_host}:${observer_port}"
echo "Recreating caddy upstream=${CREG_OBSERVER_API_UPSTREAM}"
DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then DOCKER=(sudo docker); fi
COMPOSE=("${DOCKER[@]}" compose -f docker-compose.cloud-edge.yml -f docker-compose.cloud-edge-ingress.yml --env-file sepolia-3node.env)
env CREG_CLOUD_CADDYFILE="${CREG_CLOUD_CADDYFILE}" CREG_OBSERVER_API_UPSTREAM="${CREG_OBSERVER_API_UPSTREAM}" \
  "${COMPOSE[@]}" up -d --force-recreate caddy
"${DOCKER[@]}" exec creg-cloud-caddy env | grep CREG_OBSERVER || true
curl -fsS -k "https://127.0.0.1/v1/health" -H "Host: ${CREG_PUBLIC_API_HOST:-api.testnet.cregnet.dev}"