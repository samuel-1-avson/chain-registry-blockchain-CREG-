#!/usr/bin/env bash
# First-boot setup for creg-sepolia-nethermind: Docker, the persistent data
# disk, client data directories and the Engine API JWT secret. Idempotent.
set -euo pipefail
exec > >(tee -a /var/log/creg-sepolia-nethermind-bootstrap.log) 2>&1

export DEBIAN_FRONTEND=noninteractive
apt-get update -y
apt-get install -y ca-certificates docker.io openssl
systemctl enable --now docker

device=/dev/disk/by-id/google-creg-sepolia-nethermind-data
mountpoint=/mnt/disks/sepolia-nethermind
if [[ ! -e "$device" ]]; then
  echo "Expected persistent data disk $device is missing" >&2
  exit 1
fi
if ! blkid "$device" >/dev/null 2>&1; then
  mkfs.ext4 -F "$device"
fi
mkdir -p "$mountpoint"
disk_uuid="$(blkid -s UUID -o value "$device")"
grep -q "UUID=$disk_uuid " /etc/fstab || \
  echo "UUID=$disk_uuid $mountpoint ext4 defaults,nofail 0 2" >> /etc/fstab
mountpoint -q "$mountpoint" || mount "$mountpoint"
chown root:root "$mountpoint"
chmod 0750 "$mountpoint"

# Nethermind runs as root in its image; Teku runs as uid/gid 1000.
install -d -m 0750 "$mountpoint/nethermind"
install -d -m 0750 -o 1000 -g 1000 "$mountpoint/teku"

# Shared Engine API secret, generated once on this VM and never copied off it.
if [[ ! -s "$mountpoint/jwtsecret" ]]; then
  umask 077
  openssl rand -hex 32 | tr -d '\n' > "$mountpoint/jwtsecret"
fi
chmod 0644 "$mountpoint/jwtsecret"

touch /var/lib/creg-sepolia-nethermind-bootstrap.done
