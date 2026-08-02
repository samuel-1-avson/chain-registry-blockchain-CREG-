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

# Do not clone or execute a mutable branch from instance metadata. The
# deployment operator must sync a reviewed release, push the private env, and
# explicitly run start-observer-pool-gcp.sh after the immutable image/preflight
# gate passes. This bootstrap only installs Docker and leaves the MIG stopped
# until that controlled rollout.

touch "$MARKER"
