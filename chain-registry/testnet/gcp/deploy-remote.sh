#!/usr/bin/env bash
# Runs on the GCP VM via: gcloud compute ssh ... --command "bash -s" < deploy-remote.sh
set -euo pipefail

GITHUB_REPO="${GITHUB_REPO:-samuel-1-avson/chain-registry-blockchain-CREG-}"
GITHUB_BRANCH="${GITHUB_BRANCH:-main}"
WORKDIR="${WORKDIR:-$HOME/creg-hosting}"
REPO_SLUG="${GITHUB_REPO##*/}"
REPO_ROOT="$WORKDIR/$REPO_SLUG/chain-registry"

wait_for_docker() {
  for i in $(seq 1 120); do
    if docker info >/dev/null 2>&1 || sudo docker info >/dev/null 2>&1; then
      echo "Docker ready (attempt $i)"
      return 0
    fi
    sleep 10
  done
  echo "Docker not ready after 20 minutes (check startup script on VM)" >&2
  exit 1
}

mkdir -p "$WORKDIR"
ENV_CANDIDATE="$WORKDIR/$REPO_SLUG/chain-registry/testnet/sepolia-3node.env"
ENV_BACKUP=""
if [[ -f "$ENV_CANDIDATE" ]]; then
  ENV_BACKUP="$(mktemp)"
  cp "$ENV_CANDIDATE" "$ENV_BACKUP"
  chmod 600 "$ENV_BACKUP"
fi

if [[ -d "$WORKDIR/$REPO_SLUG" && ! -d "$WORKDIR/$REPO_SLUG/.git" ]]; then
  echo "Removing non-git directory $WORKDIR/$REPO_SLUG (e.g. leftover from push-env) ..."
  rm -rf "$WORKDIR/$REPO_SLUG"
fi
if [[ ! -d "$WORKDIR/$REPO_SLUG/.git" ]]; then
  git clone "https://github.com/${GITHUB_REPO}.git" "$WORKDIR/$REPO_SLUG"
fi

if [[ -n "$ENV_BACKUP" && -f "$ENV_BACKUP" ]]; then
  mkdir -p "$(dirname "$ENV_CANDIDATE")"
  cp "$ENV_BACKUP" "$ENV_CANDIDATE"
  chmod 600 "$ENV_CANDIDATE"
  rm -f "$ENV_BACKUP"
fi

cd "$WORKDIR/$REPO_SLUG"
git fetch origin
git checkout "$GITHUB_BRANCH" 2>/dev/null || git checkout -B "$GITHUB_BRANCH" "origin/$GITHUB_BRANCH"
git pull --ff-only origin "$GITHUB_BRANCH" || true

cd "$REPO_ROOT"
if [[ ! -f testnet/sepolia-3node.env ]]; then
  echo "Missing $REPO_ROOT/testnet/sepolia-3node.env - run push-env.ps1 from workstation" >&2
  exit 1
fi
chmod 600 testnet/sepolia-3node.env

chmod +x testnet/start-3node-gcp.sh 2>/dev/null || true
wait_for_docker
bash testnet/start-3node-gcp.sh

echo ""
echo "=== Recent Caddy logs ==="
docker logs --tail 40 creg-3node-caddy 2>/dev/null || true
