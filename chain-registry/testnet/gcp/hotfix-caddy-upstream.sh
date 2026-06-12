#!/usr/bin/env bash
set -euo pipefail
REPO="${HOME}/creg-hosting/chain-registry-blockchain-CREG-/chain-registry"
cd "${REPO}/testnet"

if [[ -f /tmp/docker-compose.cloud-edge-ingress.yml ]]; then
  cp /tmp/docker-compose.cloud-edge-ingress.yml ./
  tr -d '\r' < ./docker-compose.cloud-edge-ingress.yml > ./docker-compose.cloud-edge-ingress.yml.lf
  mv ./docker-compose.cloud-edge-ingress.yml.lf ./docker-compose.cloud-edge-ingress.yml
fi

# Fallback Caddyfile: use validator IP vars already injected into the caddy container.
cat > ./caddy/Caddyfile.fleet <<'CADDY'
# Fleet layout: validators on creg-validator-vm (VPC); edge services on this VM.
{
	email {$CREG_ACME_EMAIL}
}

import waitlist.caddy
import faucet-edge.caddy
import hub.caddy

{$CREG_PUBLIC_API_HOST} {
	reverse_proxy {$CREG_VALIDATOR_VM_INTERNAL_IP}:{$CREG_3NODE_NODE3_API_PORT} {
		flush_interval -1
	}
}

{$CREG_PUBLIC_EXPLORER_HOST} {
	reverse_proxy 127.0.0.1:{$CREG_3NODE_EXPLORER_PORT} {
		flush_interval -1
	}
}

{$CREG_PUBLIC_SPEC_HOST} {
	reverse_proxy 127.0.0.1:{$CREG_3NODE_SPEC_HOST_PORT}
}

{$CREG_PUBLIC_IPFS_HOST} {
	reverse_proxy 127.0.0.1:{$CREG_3NODE_IPFS_HOST_PORT}
}
CADDY

# shellcheck source=testnet/_source-sepolia-env.sh
source ./_source-sepolia-env.sh
creg_source_sepolia_env "./sepolia-3node.env"

export CREG_CLOUD_CADDYFILE="./caddy/Caddyfile.fleet"
observer_host="${CREG_OBSERVER_POOL_LB_IP:-${CREG_VALIDATOR_VM_INTERNAL_IP}}"
observer_port="${CREG_3NODE_NODE3_API_PORT:-28182}"
export CREG_OBSERVER_API_UPSTREAM="${observer_host}:${observer_port}"
echo "Recreating caddy — API upstream ${CREG_OBSERVER_API_UPSTREAM} (Caddyfile uses validator IP:port until compose env is synced)"

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  DOCKER=(sudo docker)
fi

COMPOSE=("${DOCKER[@]}" compose -f docker-compose.cloud-edge.yml -f docker-compose.cloud-edge-ingress.yml --env-file sepolia-3node.env)
env CREG_CLOUD_CADDYFILE="${CREG_CLOUD_CADDYFILE}" CREG_OBSERVER_API_UPSTREAM="${CREG_OBSERVER_API_UPSTREAM}" \
  "${COMPOSE[@]}" up -d --force-recreate caddy

"${DOCKER[@]}" exec creg-cloud-caddy env | grep CREG_OBSERVER_API_UPSTREAM || true
curl -fsS "http://127.0.0.1:443/v1/health" -k -H "Host: api.testnet.cregnet.dev" || true
