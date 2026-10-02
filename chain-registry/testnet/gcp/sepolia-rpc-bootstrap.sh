#!/usr/bin/env bash
set -euo pipefail
exec > >(tee -a /var/log/creg-sepolia-rpc-bootstrap.log) 2>&1

export DEBIAN_FRONTEND=noninteractive
apt-get update -y
apt-get install -y ca-certificates docker.io
systemctl enable --now docker

device=/dev/disk/by-id/google-creg-sepolia-rpc-data
mountpoint=/mnt/disks/sepolia-rpc
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
touch /var/lib/creg-sepolia-rpc-bootstrap.done
