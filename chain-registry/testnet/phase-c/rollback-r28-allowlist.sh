#!/usr/bin/env bash
# Undo cutover-r28-allowlist.sh: restore the Caddy files saved under STAMP, reload
# Caddy, and bring back the explorer container that was running before.
#
# Usage (on creg-testnet-vm): bash rollback-r28-allowlist.sh STAMP
set -euo pipefail

stamp="${1:?the stamp printed by cutover-r28-allowlist.sh}"
repo="${CREG_REPO:-$HOME/creg-hosting/chain-registry-blockchain-CREG-}"
caddy_dir="$repo/chain-registry/testnet/caddy"
explorer=creg-explorer-public-r28
previous=creg-explorer-public-r28-before-allowlist
caddy=creg-unified-caddy

DOCKER=(docker)
docker info >/dev/null 2>&1 || DOCKER=(sudo docker)
say() { echo "[rollback] $*"; }

for name in Caddyfile.unified sepolia-public-rpc.caddy; do
  backup="$caddy_dir/$name.pre-rpc-phase1-$stamp"
  [[ -s "$backup" ]] || { echo "[rollback] missing $backup" >&2; exit 1; }
  cat "$backup" > "$caddy_dir/$name"
done
"${DOCKER[@]}" exec "$caddy" caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null
"${DOCKER[@]}" exec "$caddy" caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null
say "Caddy files from $stamp restored and reloaded"

if "${DOCKER[@]}" inspect "$previous" >/dev/null 2>&1; then
  "${DOCKER[@]}" stop --time 10 "$explorer" >/dev/null 2>&1 || true
  "${DOCKER[@]}" rename "$explorer" "$explorer-allowlist-rolled-back-$stamp" 2>/dev/null || true
  "${DOCKER[@]}" update --restart no "$explorer-allowlist-rolled-back-$stamp" >/dev/null 2>&1 || true
  "${DOCKER[@]}" rename "$previous" "$explorer"
  "${DOCKER[@]}" update --restart unless-stopped "$explorer" >/dev/null
  "${DOCKER[@]}" start "$explorer" >/dev/null
  say "previous $explorer restored (the allowlisted one is kept, stopped, as $explorer-allowlist-rolled-back-$stamp)"
else
  say "no $previous container; explorer left as it is"
fi
say "ROLLBACK-OK"
