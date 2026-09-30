#!/usr/bin/env bash
# Base OS preparation for a real Phase C validator or observer VM.
# This script does not install or start a CREG node.
set -euo pipefail

device=/dev/disk/by-id/google-creg-data
mount_dir=/var/lib/creg

for _ in $(seq 1 60); do
  if [ -b "$device" ]; then
    break
  fi
  sleep 2
done
if [ ! -b "$device" ]; then
  echo "CREG data disk was not attached: $device" >&2
  exit 1
fi

if ! blkid "$device" >/dev/null 2>&1; then
  mkfs.ext4 -F -L CREG_DATA "$device"
fi
mkdir -p "$mount_dir"
disk_uuid=$(blkid -s UUID -o value "$device")
if ! grep -q "^UUID=$disk_uuid " /etc/fstab; then
  printf 'UUID=%s %s ext4 defaults,nofail 0 2\n' "$disk_uuid" "$mount_dir" >> /etc/fstab
fi
if ! mountpoint -q "$mount_dir"; then
  mount "$mount_dir"
fi
chmod 0700 "$mount_dir"

if ! command -v docker >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  apt-get install -y docker.io ca-certificates curl jq
fi
systemctl enable --now docker
