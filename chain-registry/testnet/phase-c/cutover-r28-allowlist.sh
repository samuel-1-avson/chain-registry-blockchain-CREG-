#!/usr/bin/env bash
# RPC remediation Phase 1, cutover (docs/rpc-remediation-plan.md, finding R-01).
#
# 1. Recreates creg-explorer-public-r28 with a real Sepolia upstream behind its
#    allowlisted /rpc gateway. The previous container is kept, stopped, as
#    creg-explorer-public-r28-before-allowlist.
# 2. Installs the new Caddyfile.unified and sepolia-public-rpc.caddy (backups are
#    kept next to them), validates and reloads Caddy, so /sepolia, explorer /rpc
#    and faucet /rpc go through that gateway and /metrics is refused.
#
# A failure before the Caddy reload restores what it changed. To undo a finished
# cutover: bash rollback-r28-allowlist.sh STAMP (the stamp is printed at the end).
#
# Usage (on creg-testnet-vm):
#   bash cutover-r28-allowlist.sh UPSTREAM_URL UPSTREAM_HOST NEW_CADDYFILE NEW_SEPOLIA_CADDY
set -euo pipefail

upstream="${1:?upstream URL}"
upstream_host="${2:?upstream Host header}"
new_caddyfile="${3:?path to the new Caddyfile.unified}"
new_sepolia="${4:?path to the new sepolia-public-rpc.caddy}"

repo="${CREG_REPO:-$HOME/creg-hosting/chain-registry-blockchain-CREG-}"
caddy_dir="$repo/chain-registry/testnet/caddy"
explorer=creg-explorer-public-r28
previous=creg-explorer-public-r28-before-allowlist
stage=creg-explorer-public-r28-allowlist-stage
caddy=creg-unified-caddy
prometheus=creg-gcp-prometheus
network=testnet_creg-unified-net
stamp=$(date -u +%Y%m%dT%H%M%SZ)

DOCKER=(docker)
docker info >/dev/null 2>&1 || DOCKER=(sudo docker)
say() { echo "[cutover] $*"; }
die() { echo "[cutover] ABORT: $*" >&2; exit 1; }

# ── Preconditions (nothing has changed yet) ──────────────────────────────────
case "$upstream" in
  *cregnet.dev*) die "the upstream must be a real Sepolia RPC, never the CREG edge" ;;
esac
"${DOCKER[@]}" inspect "$explorer" >/dev/null 2>&1 || die "no $explorer container"
! "${DOCKER[@]}" inspect "$previous" >/dev/null 2>&1 || die "$previous already exists; finish or roll back the earlier cutover first"
[[ -s "$new_caddyfile" && -s "$new_sepolia" ]] || die "new Caddy files are missing"
grep -q 'creg-explorer-public-r28:80' "$new_caddyfile" || die "new Caddyfile does not route to $explorer"
grep -q 'allowlisted' "$new_sepolia" || die "new sepolia-public-rpc.caddy is not the ECA-38 version"
if grep -q $'\r' "$new_caddyfile" "$new_sepolia"; then die "new Caddy files have CRLF line endings"; fi
if "${DOCKER[@]}" inspect "$prometheus" >/dev/null 2>&1; then
  if "${DOCKER[@]}" exec "$prometheus" sh -c 'cat /etc/prometheus/*.yml' 2>/dev/null | grep -q 'cregnet.dev'; then
    die "Prometheus scrapes a public cregnet.dev host; /metrics will be refused there"
  fi
fi

image=$("${DOCKER[@]}" inspect -f '{{.Config.Image}}' "$explorer")
restart=$("${DOCKER[@]}" inspect -f '{{.HostConfig.RestartPolicy.Name}}' "$explorer")
[[ -n "$restart" && "$restart" != "no" ]] || restart=unless-stopped
bind=$("${DOCKER[@]}" inspect -f '{{range $p, $c := .HostConfig.PortBindings}}{{range $c}}{{.HostIp}}:{{.HostPort}}{{end}}{{end}}' "$explorer")
[[ -n "$bind" ]] || die "could not read $explorer's port binding"
"${DOCKER[@]}" run --rm --entrypoint test "$image" -f /etc/nginx/njs/rpc-allowlist.js || die "$image has no rpc-allowlist.js"
mapfile -t envs < <("${DOCKER[@]}" inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$explorer" \
  | grep -E '^CREG_' | grep -vE '^CREG_SEPOLIA_RPC_(UPSTREAM|HOST)=')
env_args=()
for entry in "${envs[@]}"; do [[ -n "$entry" ]] && env_args+=(-e "$entry"); done
say "image $image, restart $restart, bind $bind, settings: $(printf '%s\n' "${envs[@]}" | cut -d= -f1 | paste -sd' ' -)"

restore_explorer() {
  say "restoring the previous $explorer"
  "${DOCKER[@]}" rm -f "$explorer" >/dev/null 2>&1 || true
  "${DOCKER[@]}" rename "$previous" "$explorer"
  "${DOCKER[@]}" update --restart "$restart" "$explorer" >/dev/null
  "${DOCKER[@]}" start "$explorer" >/dev/null
}

# ── Step 1: explorer with a real upstream behind its allowlist ──────────────
say "step 1: recreating $explorer"
"${DOCKER[@]}" stop --time 10 "$explorer" >/dev/null
"${DOCKER[@]}" rename "$explorer" "$previous"
"${DOCKER[@]}" update --restart no "$previous" >/dev/null
if ! "${DOCKER[@]}" run -d --name "$explorer" --restart "$restart" --network "$network" \
    -p "$bind:80" "${env_args[@]}" \
    -e "CREG_SEPOLIA_RPC_UPSTREAM=$upstream" -e "CREG_SEPOLIA_RPC_HOST=$upstream_host" \
    "$image" >/dev/null; then
  restore_explorer
  die "the new $explorer did not start"
fi

base="http://$bind"
ready=0
for _ in $(seq 1 30); do
  if curl -fsS -o /dev/null "$base/"; then ready=1; break; fi
  sleep 1
done
rpc() { curl -s -o /dev/null -w '%{http_code}' -X POST "$base/rpc" -H 'content-type: application/json' --data-binary "$1"; }
chain=$(curl -s -X POST "$base/rpc" -H 'content-type: application/json' --data-binary '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' || true)
logs=$(rpc '{"jsonrpc":"2.0","method":"eth_getLogs","params":[{"fromBlock":"latest"}],"id":2}')
if [[ $ready -ne 1 || "$chain" != *0xaa36a7* || "$logs" != 403 ]]; then
  restore_explorer
  die "the new $explorer failed its checks (ready=$ready, eth_getLogs=$logs)"
fi
say "step 1 done: explorer page up, eth_chainId answered, eth_getLogs refused"

# ── Step 2: Caddy routes the Sepolia surfaces through the gateway ───────────
say "step 2: installing Caddy files (backups use suffix .pre-rpc-phase1-$stamp)"
cp -p "$caddy_dir/Caddyfile.unified" "$caddy_dir/Caddyfile.unified.pre-rpc-phase1-$stamp"
cp -p "$caddy_dir/sepolia-public-rpc.caddy" "$caddy_dir/sepolia-public-rpc.caddy.pre-rpc-phase1-$stamp"
# Write in place: the container bind-mounts these files, and a new inode
# (mv, editor save) would not be seen by Caddy.
cat "$new_caddyfile" > "$caddy_dir/Caddyfile.unified"
cat "$new_sepolia" > "$caddy_dir/sepolia-public-rpc.caddy"
restore_caddy_files() {
  cat "$caddy_dir/Caddyfile.unified.pre-rpc-phase1-$stamp" > "$caddy_dir/Caddyfile.unified"
  cat "$caddy_dir/sepolia-public-rpc.caddy.pre-rpc-phase1-$stamp" > "$caddy_dir/sepolia-public-rpc.caddy"
}
if ! "${DOCKER[@]}" exec "$caddy" caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null 2>&1; then
  "${DOCKER[@]}" exec "$caddy" caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile 2>&1 | tail -n 5 || true
  restore_caddy_files
  die "caddy validate failed; Caddy files restored and not reloaded. The explorer is already recreated (it serves the same pages); bash rollback-r28-allowlist.sh $stamp restores it too"
fi
if ! "${DOCKER[@]}" exec "$caddy" caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null 2>&1; then
  restore_caddy_files
  "${DOCKER[@]}" exec "$caddy" caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null 2>&1 || true
  die "caddy reload failed; previous Caddy files restored and reloaded. Explorer rollback: bash rollback-r28-allowlist.sh $stamp"
fi
say "step 2 done: Caddy reloaded"

"${DOCKER[@]}" rm -f "$stage" >/dev/null 2>&1 || true
say "CUTOVER-OK stamp=$stamp (rollback: bash rollback-r28-allowlist.sh $stamp)"
