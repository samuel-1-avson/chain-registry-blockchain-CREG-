#!/usr/bin/env bash
# GCP edge-only stack: Caddy, explorer, faucet, IPFS, spec, waitlist — no CREG nodes on this VM.
#
# Modes (sepolia-3node.env):
#   CREG_VALIDATOR_FLEET_MODE=true  — validators on creg-validator-vm (production)
#   CREG_HYBRID_MODE=true           — validators on operator PC via WireGuard (legacy)
#
# Usage:
#   ./testnet/start-cloud-edge-gcp.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

ENV_FILE="${SCRIPT_DIR}/sepolia-3node.env"
if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE" >&2
  exit 1
fi

# shellcheck source=testnet/_source-sepolia-env.sh
source "${SCRIPT_DIR}/_source-sepolia-env.sh"
creg_source_sepolia_env "$ENV_FILE"

export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.hybrid"
export CREG_EXPLORER_NGINX_CONF="./nginx/explorer-hybrid.conf"

if [[ "${CREG_VALIDATOR_FLEET_MODE:-false}" == "true" ]]; then
  if [[ -z "${CREG_VALIDATOR_VM_INTERNAL_IP:-}" ]]; then
    echo "Set CREG_VALIDATOR_VM_INTERNAL_IP in sepolia-3node.env" >&2
    exit 1
  fi
  export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.fleet"
  FLEET_NGINX="${SCRIPT_DIR}/nginx/explorer-fleet.conf"
  sed "s/@VALIDATOR_IP@/${CREG_VALIDATOR_VM_INTERNAL_IP}/g" \
    "${SCRIPT_DIR}/nginx/explorer-fleet.conf.template" > "$FLEET_NGINX"
  export CREG_EXPLORER_NGINX_CONF="./nginx/explorer-fleet.conf"
  echo "Validator fleet mode — API/explorer -> ${CREG_VALIDATOR_VM_INTERNAL_IP}"
elif [[ "${CREG_HYBRID_MODE:-false}" == "true" ]]; then
  echo "Hybrid mode — API -> WireGuard peer ${CREG_WG_LOCAL_PEER:-10.200.0.2}"
else
  echo "Warning: neither CREG_VALIDATOR_FLEET_MODE nor CREG_HYBRID_MODE set; using hybrid Caddy defaults"
fi

CADDY_DIR="${SCRIPT_DIR}/caddy"
if [[ -n "${CREG_PUBLIC_WAITLIST_HOST:-}" ]]; then
  cp "${CADDY_DIR}/waitlist-edge.caddy.example" "${CADDY_DIR}/waitlist.caddy"
  echo "Activated caddy/waitlist.caddy for ${CREG_PUBLIC_WAITLIST_HOST}"
fi
if [[ -n "${CREG_PUBLIC_FAUCET_HOST:-}" ]]; then
  cp "${CADDY_DIR}/faucet-edge.caddy.example" "${CADDY_DIR}/faucet-edge.caddy"
  echo "Activated caddy/faucet-edge.caddy for ${CREG_PUBLIC_FAUCET_HOST}"
fi

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  if sudo docker info >/dev/null 2>&1; then
    DOCKER=(sudo -E docker)
  else
    echo "Docker not available" >&2
    exit 1
  fi
fi

COMPOSE_FILES=(
  "${SCRIPT_DIR}/docker-compose.cloud-edge.yml"
  "${SCRIPT_DIR}/docker-compose.cloud-edge-ingress.yml"
)

if [[ -f "${SCRIPT_DIR}/waitlist/dist/index.html" ]]; then
  echo "Waitlist dist found — enabling waitlist profile"
  export COMPOSE_PROFILES=waitlist
fi

COMPOSE_ARGS=()
for compose_file in "${COMPOSE_FILES[@]}"; do
  COMPOSE_ARGS+=(-f "$compose_file")
done
COMPOSE=("${DOCKER[@]}" compose "${COMPOSE_ARGS[@]}" --env-file "$ENV_FILE")
# sudo may drop exported vars; pass ingress paths explicitly for compose interpolation.
COMPOSE_ENV=(
  CREG_CLOUD_CADDYFILE="${CREG_CLOUD_CADDYFILE}"
  CREG_EXPLORER_NGINX_CONF="${CREG_EXPLORER_NGINX_CONF}"
)
compose_run() {
  env "${COMPOSE_ENV[@]}" "${COMPOSE[@]}" "$@"
}

echo "=== Building faucet image (creg-node) ==="
compose_run build faucet

echo "=== Stopping legacy full-stack containers if present ==="
"${DOCKER[@]}" rm -f creg-3node-node1 creg-3node-node2 creg-3node-node3 creg-3node-caddy \
  creg-fleet-node1 creg-fleet-node2 creg-fleet-node3 2>/dev/null || true

echo "=== Starting cloud edge stack ==="
compose_run down --remove-orphans 2>/dev/null || true
compose_run up -d --build --remove-orphans

echo ""
echo "Cloud edge up."
echo "  docker logs -f creg-cloud-caddy"
if [[ -n "${CREG_PUBLIC_API_HOST:-}" ]]; then
  echo "  curl -fsS https://${CREG_PUBLIC_API_HOST}/v1/health"
fi
