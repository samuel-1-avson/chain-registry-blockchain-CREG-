#!/usr/bin/env bash
# GCP public-services stack: Caddy, explorer, faucet, IPFS, spec, waitlist.
# Validators and their keys are operated independently outside GCP.
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
BOOTSTRAP_REQUESTED="${CREG_PUBLIC_SERVICES_BOOTSTRAP:-}"
source "${SCRIPT_DIR}/_source-sepolia-env.sh"
creg_source_sepolia_env "$ENV_FILE"

# A deploy-stack -BootstrapOnly request is captured before sourcing the env so
# a stale value in a copied file cannot downgrade the requested mode. An env
# file may also opt into bootstrap mode directly for manual rehearsals.
if [[ -z "$BOOTSTRAP_REQUESTED" ]]; then
  BOOTSTRAP_REQUESTED="${CREG_PUBLIC_SERVICES_BOOTSTRAP:-false}"
fi

# Resolve the faucet key only in the VM process from Google Secret Manager.
# The env file carries a secret ID, never private material. The VM service
# account must have Secret Manager Secret Accessor and cloud-platform scope.
if [[ -z "${FAUCET_PRIVATE_KEY:-}" && -n "${CREG_FAUCET_SECRET_ID:-}" ]]; then
  if [[ -z "${CREG_SECRET_PROJECT_ID:-}" ]]; then
    echo "CREG_SECRET_PROJECT_ID is required when CREG_FAUCET_SECRET_ID is set" >&2
    exit 1
  fi
  metadata_token_json="$(curl -fsS -H 'Metadata-Flavor: Google' \
    'http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token')"
  metadata_token="$(printf '%s' "$metadata_token_json" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')"
  if [[ -z "$metadata_token" ]]; then
    echo "Could not obtain a GCE metadata access token for faucet Secret Manager access" >&2
    exit 1
  fi
  secret_json="$(curl -fsS -H "Authorization: Bearer ${metadata_token}" \
    "https://secretmanager.googleapis.com/v1/projects/${CREG_SECRET_PROJECT_ID}/secrets/${CREG_FAUCET_SECRET_ID}/versions/latest:access")"
  secret_b64="$(printf '%s' "$secret_json" | sed -n 's/.*"data"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
  if [[ -z "$secret_b64" ]]; then
    echo "Secret Manager returned no faucet payload for ${CREG_FAUCET_SECRET_ID}" >&2
    exit 1
  fi
  export FAUCET_PRIVATE_KEY="$(printf '%s' "$secret_b64" | base64 --decode | tr -d '\r\n')"
  if [[ ! "$FAUCET_PRIVATE_KEY" =~ ^(0x)?[0-9a-fA-F]{64}$ ]]; then
    echo "Fetched faucet secret is not a 32-byte secp256k1 key" >&2
    unset FAUCET_PRIVATE_KEY
    exit 1
  fi
  echo "Faucet signing key loaded from Secret Manager (${CREG_FAUCET_SECRET_ID}); private material is not logged"
fi

if [[ "${CREG_VALIDATOR_FLEET_MODE:-false}" == "true" || "${CREG_HYBRID_MODE:-false}" == "true" ]]; then
  echo "CREG_VALIDATOR_FLEET_MODE/CREG_HYBRID_MODE are retired for the GCP public-services profile; validators must run independently." >&2
  exit 1
fi

if [[ "$BOOTSTRAP_REQUESTED" == "true" || "$BOOTSTRAP_REQUESTED" == "1" || "$BOOTSTRAP_REQUESTED" == "yes" ]]; then
  export CREG_PUBLIC_SERVICES_BOOTSTRAP=true
  export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.bootstrap"
  export CREG_EXPLORER_NGINX_CONF="./nginx/explorer-bootstrap.conf"
  export CREG_OBSERVER_API_UPSTREAM=""
  echo "Bootstrap mode — spec/IPFS/explorer/faucet may start before an observer or validator is online"
else
  export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.fleet"
  observer_host="${CREG_OBSERVER_POOL_LB_IP:-}"
  observer_port="${CREG_3NODE_NODE3_API_PORT:-28182}"
  export CREG_OBSERVER_API_UPSTREAM="${CREG_OBSERVER_API_UPSTREAM:-${observer_host:+${observer_host}:${observer_port}}}"
  CREG_OBSERVER_API_UPSTREAM="${CREG_OBSERVER_API_UPSTREAM#http://}"
  CREG_OBSERVER_API_UPSTREAM="${CREG_OBSERVER_API_UPSTREAM#https://}"
  export CREG_OBSERVER_API_UPSTREAM
  if [[ -z "${CREG_OBSERVER_API_UPSTREAM:-}" ]]; then
    echo "Set CREG_OBSERVER_API_UPSTREAM or CREG_OBSERVER_POOL_LB_IP in sepolia-3node.env; the GCP edge never proxies to a validator VM." >&2
    exit 1
  fi
  FLEET_NGINX="${SCRIPT_DIR}/nginx/explorer-fleet.conf"
  sed -e "s/@OBSERVER_UPSTREAM@/${CREG_OBSERVER_API_UPSTREAM}/g" \
    "${SCRIPT_DIR}/nginx/explorer-fleet.conf.template" > "$FLEET_NGINX"
  export CREG_EXPLORER_NGINX_CONF="./nginx/explorer-fleet.conf"
  echo "External-validator mode — public API -> ${CREG_OBSERVER_API_UPSTREAM}"
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

# Sepolia L1 JSON-RPC shown in faucet UI (eth_*). Never point at api.* /rpc (creg_* only).
if [[ -z "${FAUCET_PUBLIC_RPC_URL:-}" && -n "${CREG_PUBLIC_EXPLORER_HOST:-}" ]]; then
  export FAUCET_PUBLIC_RPC_URL="https://${CREG_PUBLIC_EXPLORER_HOST}/rpc"
fi
if [[ -z "${CREG_SEPOLIA_PUBLIC_RPC_UPSTREAM:-}" ]]; then
  export CREG_SEPOLIA_PUBLIC_RPC_UPSTREAM="https://ethereum-sepolia-rpc.publicnode.com"
fi
if [[ -z "${CREG_SEPOLIA_PUBLIC_RPC_UPSTREAM_HOST:-}" ]]; then
  export CREG_SEPOLIA_PUBLIC_RPC_UPSTREAM_HOST="ethereum-sepolia-rpc.publicnode.com"
fi
if [[ -n "${CREG_PUBLIC_JOIN_HOST:-}" ]]; then
  cp "${CADDY_DIR}/hub-edge.caddy.example" "${CADDY_DIR}/hub.caddy"
  echo "Activated caddy/hub.caddy for ${CREG_PUBLIC_JOIN_HOST}"
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

COMPOSE_PROFILE_LIST=()
if [[ -f "${SCRIPT_DIR}/waitlist/dist/index.html" ]]; then
  echo "Waitlist dist found — enabling waitlist profile"
  COMPOSE_PROFILE_LIST+=(waitlist)
fi
if [[ -n "${CREG_PUBLIC_JOIN_HOST:-}" ]]; then
  echo "Join hub enabled — enabling hub profile (${CREG_PUBLIC_JOIN_HOST})"
  COMPOSE_PROFILE_LIST+=(hub)
  if [[ -n "${CREG_HUB_API_CLOUD_RUN_URL:-}" ]]; then
    echo "Hub API on Cloud Run (${CREG_HUB_API_CLOUD_RUN_URL}) — edge runs hub-web only"
    export CREG_HUB_EDGE_API_MODE=cloudrun
  fi
fi
if [[ ${#COMPOSE_PROFILE_LIST[@]} -gt 0 ]]; then
  export COMPOSE_PROFILES="$(IFS=,; echo "${COMPOSE_PROFILE_LIST[*]}")"
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
  CREG_OBSERVER_API_UPSTREAM="${CREG_OBSERVER_API_UPSTREAM:-}"
  CREG_PUBLIC_SERVICES_BOOTSTRAP="${CREG_PUBLIC_SERVICES_BOOTSTRAP:-false}"
)
compose_run() {
  env "${COMPOSE_ENV[@]}" "${COMPOSE[@]}" "$@"
}

if [[ "${CREG_SKIP_FAUCET_BUILD:-false}" == "true" || "${CREG_SKIP_FAUCET_BUILD:-false}" == "1" ]]; then
  echo "=== Reusing the already-built faucet image (explicit skip) ==="
else
  echo "=== Building faucet image (creg-node) ==="
  compose_run build faucet
fi

echo "=== Stopping legacy full-stack containers if present ==="
"${DOCKER[@]}" rm -f creg-3node-node1 creg-3node-node2 creg-3node-node3 creg-3node-caddy \
  creg-fleet-node1 creg-fleet-node2 creg-fleet-node3 2>/dev/null || true

echo "=== Starting cloud edge stack ==="
compose_run down --remove-orphans 2>/dev/null || true
# Fixed container_name + profile drift leaves orphans compose down does not remove.
mapfile -t stale_ids < <("${DOCKER[@]}" ps -aq --filter 'name=creg-cloud-' 2>/dev/null || true)
if [[ ${#stale_ids[@]} -gt 0 ]]; then
  "${DOCKER[@]}" rm -f "${stale_ids[@]}" 2>/dev/null || true
fi
UP_ARGS=(up -d --remove-orphans --force-recreate)
if [[ "${CREG_SKIP_FAUCET_BUILD:-false}" != "true" && "${CREG_SKIP_FAUCET_BUILD:-false}" != "1" ]]; then
  UP_ARGS+=(--build)
fi
if [[ -n "${CREG_HUB_API_CLOUD_RUN_URL:-}" ]]; then
  UP_ARGS+=(--scale hub-api=0)
fi
compose_run "${UP_ARGS[@]}"

echo ""
echo "Cloud edge up."
echo "  docker logs -f creg-cloud-caddy"
if [[ -n "${CREG_PUBLIC_API_HOST:-}" ]]; then
  echo "  curl -fsS https://${CREG_PUBLIC_API_HOST}/v1/health"
fi
if [[ -n "${CREG_PUBLIC_JOIN_HOST:-}" ]]; then
  echo "  curl -fsS https://${CREG_PUBLIC_JOIN_HOST}/api/health"
fi
