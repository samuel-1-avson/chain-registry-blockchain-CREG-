#!/usr/bin/env bash
# WireGuard hub on creg-testnet-vm — routes local validators to internal Geth + exposes API to Caddy.
# Run once on the VM (as root), from synced repo root:
#   cd ~/creg-hosting/chain-registry-blockchain-CREG-/chain-registry
#   sudo bash testnet/gcp/wireguard/install-server.sh
set -euo pipefail

WG_IF=wg0
WG_PORT=51820
WG_NET=10.200.0.0/24
WG_SERVER_IP=10.200.0.1/24
CONF_DIR=/etc/wireguard
KEY_DIR="$CONF_DIR/keys"

if [[ $(id -u) -ne 0 ]]; then
  echo "Run as root (sudo)" >&2
  exit 1
fi

apt-get update -y
apt-get install -y wireguard iptables

mkdir -p "$KEY_DIR"
chmod 700 "$CONF_DIR" "$KEY_DIR"

if [[ ! -f "$KEY_DIR/server_private" ]]; then
  umask 077
  wg genkey | tee "$KEY_DIR/server_private" | wg pubkey > "$KEY_DIR/server_public"
fi

SERVER_PRIV=$(cat "$KEY_DIR/server_private")
SERVER_PUB=$(cat "$KEY_DIR/server_public")

# Client keys (operator workstation) — regenerate client conf if re-run.
if [[ ! -f "$KEY_DIR/client_private" ]]; then
  wg genkey | tee "$KEY_DIR/client_private" | wg pubkey > "$KEY_DIR/client_public"
fi
CLIENT_PRIV=$(cat "$KEY_DIR/client_private")
CLIENT_PUB=$(cat "$KEY_DIR/client_public")

cat >"$CONF_DIR/${WG_IF}.conf" <<EOF
[Interface]
Address = ${WG_SERVER_IP}
ListenPort = ${WG_PORT}
PrivateKey = ${SERVER_PRIV}
PostUp = sysctl -w net.ipv4.ip_forward=1; iptables -A FORWARD -i %i -j ACCEPT; iptables -A FORWARD -o %i -j ACCEPT; iptables -t nat -A POSTROUTING -o ens4 -j MASQUERADE
PostDown = iptables -D FORWARD -i %i -j ACCEPT; iptables -D FORWARD -o %i -j ACCEPT; iptables -t nat -D POSTROUTING -o ens4 -j MASQUERADE

[Peer]
PublicKey = ${CLIENT_PUB}
AllowedIPs = 10.200.0.2/32
EOF

chmod 600 "$CONF_DIR/${WG_IF}.conf"
systemctl enable wg-quick@${WG_IF}
systemctl restart wg-quick@${WG_IF}

PUBLIC_IP=$(curl -fsS -H "Metadata-Flavor: Google" http://metadata.google.internal/computeMetadata/v1/instance/network-interfaces/0/access-configs/0/external-ip 2>/dev/null || true)

CLIENT_CONF="/root/creg-wireguard-client.conf"
cat >"$CLIENT_CONF" <<EOF
# Import into WireGuard for Windows (operator PC)
[Interface]
PrivateKey = ${CLIENT_PRIV}
Address = 10.200.0.2/24
DNS = 8.8.8.8

[Peer]
PublicKey = ${SERVER_PUB}
Endpoint = ${PUBLIC_IP:-YOUR_VM_PUBLIC_IP}:${WG_PORT}
AllowedIPs = 10.200.0.0/24, 10.128.0.0/9
PersistentKeepalive = 25
EOF
chmod 600 "$CLIENT_CONF"

echo ""
echo "WireGuard server ready on ${WG_IF} (${WG_SERVER_IP})"
echo "Client config written to: ${CLIENT_CONF}"
echo "Copy to workstation: gcloud compute scp creg-testnet-vm:${CLIENT_CONF} ./creg-wireguard-client.conf --tunnel-through-iap"
echo "Open UDP ${WG_PORT} on GCP firewall (tag creg-testnet) if not already."
