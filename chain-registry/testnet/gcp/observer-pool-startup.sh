#!/usr/bin/env bash
# GCE startup script for observer-pool MIG instances.
set -euo pipefail

MARKER=/var/log/creg-observer-pool-bootstrap.done
if [[ -f "$MARKER" ]]; then
  exit 0
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update -y
apt-get install -y ca-certificates curl git

if ! command -v docker >/dev/null 2>&1; then
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
  echo \
    "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu \
    $(. /etc/os-release && echo "${VERSION_CODENAME}") stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update -y
  apt-get install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
fi

systemctl enable docker
systemctl start docker

META=http://metadata.google.internal/computeMetadata/v1/instance/attributes
HDR=Metadata-Flavor:Google

GITHUB_REPO="$(curl -fs -H "$HDR" "${META}/github-repo" 2>/dev/null || true)"
GITHUB_BRANCH="$(curl -fs -H "$HDR" "${META}/github-branch" 2>/dev/null || true)"
GITHUB_REPO="${GITHUB_REPO:-samuel-1-avson/chain-registry-blockchain-CREG-}"
GITHUB_BRANCH="${GITHUB_BRANCH:-main}"

CLONE_DIR=/opt/chain-registry
if [[ ! -d "$CLONE_DIR/.git" ]]; then
  rm -rf "$CLONE_DIR"
  git clone --depth 1 --branch "$GITHUB_BRANCH" "https://github.com/${GITHUB_REPO}.git" "$CLONE_DIR"
fi

cd "$CLONE_DIR/chain-registry"
chmod +x testnet/start-observer-pool-gcp.sh
./testnet/start-observer-pool-gcp.sh || true

touch "$MARKER"
