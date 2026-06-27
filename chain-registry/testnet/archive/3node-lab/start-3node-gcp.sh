#!/usr/bin/env bash
# Start 3-node Sepolia fleet + public services + Caddy TLS ingress on Linux (GCP VM).
#
# Usage:
#   export BASE_DOMAIN=testnet.example.com
#   ./testnet/start-3node-gcp.sh
#
# Requires: testnet/sepolia-3node.env with CREG_PUBLIC_* and CREG_ACME_EMAIL set
# (run patch-sepolia-chain-spec-services.ps1 -BaseDomain first, or use pwsh on the VM).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

ENV_FILE="${SCRIPT_DIR}/sepolia-3node.env"
if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE — copy from sepolia-3node.env.example" >&2
  exit 1
fi

# shellcheck source=testnet/_source-sepolia-env.sh
source "${SCRIPT_DIR}/_source-sepolia-env.sh"
creg_source_sepolia_env "$ENV_FILE"

# GCE SSH users may not be in the docker group until re-login; sudo works immediately.
DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  if sudo docker info >/dev/null 2>&1; then
    DOCKER=(sudo docker)
    echo "Using sudo for Docker (add user to group docker to avoid this)"
  else
    echo "Docker is not available" >&2
    exit 1
  fi
fi

COMPOSE_FILES=(
  "${SCRIPT_DIR}/docker-compose.3node.yml"
  "${SCRIPT_DIR}/docker-compose.3node-services.yml"
)
if [[ -f "${SCRIPT_DIR}/waitlist/dist/index.html" ]]; then
  COMPOSE_FILES+=("${SCRIPT_DIR}/docker-compose.waitlist.yml")
  echo "Waitlist dist found — including marketing site (docker-compose.waitlist.yml)"
  if [[ -n "${CREG_PUBLIC_WAITLIST_HOST:-}" ]]; then
    cp "${SCRIPT_DIR}/caddy/waitlist.caddy.example" "${SCRIPT_DIR}/caddy/waitlist.caddy"
    echo "Activated caddy/waitlist.caddy for ${CREG_PUBLIC_WAITLIST_HOST}"
  fi
fi
COMPOSE_FILES+=("${SCRIPT_DIR}/docker-compose.3node-ingress.yml")

COMPOSE_ARGS=()
for compose_file in "${COMPOSE_FILES[@]}"; do
  COMPOSE_ARGS+=(-f "$compose_file")
done
COMPOSE=("${DOCKER[@]}" compose "${COMPOSE_ARGS[@]}" --env-file "$ENV_FILE")

echo "=== Building creg-node image ==="
"${COMPOSE[@]}" build creg-node-1

echo "=== Starting full public stack ==="
# Clean partial deploys (container name conflicts, unhealthy deps) before recreate.
"${COMPOSE[@]}" down --remove-orphans 2>/dev/null || true
"${COMPOSE[@]}" up -d --build --remove-orphans

echo ""
echo "Stack started. Watch TLS issuance:"
echo "  docker logs -f creg-3node-caddy"
echo ""
echo "Health (after certs):"
creg_source_sepolia_env "$ENV_FILE" 2>/dev/null || true
if [[ -n "${CREG_PUBLIC_API_HOST:-}" ]]; then
  echo "  curl -fsS https://${CREG_PUBLIC_API_HOST}/v1/health"
fi
