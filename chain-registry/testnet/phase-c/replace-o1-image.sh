#!/usr/bin/env bash
# RPC remediation Phase 2 deploy (docs/rpc-remediation-plan.md): replace observer o1
# (creg-phase-c-r7-o1) with a new node image. Its configuration is cloned from the
# running container: env, mounts, published ports, networks and alias, user and
# restart policy. Only NAME=VALUE settings given on the command line are added or
# replaced.
#
# The previous container is kept, stopped, as creg-phase-c-r7-o1-before-STAMP. Both
# share the data directory, so only one may run at a time. Any failed check after the
# swap restores the previous container. To undo a finished swap:
#   bash rollback-o1-image.sh STAMP
#
# Usage (on creg-testnet-vm):
#   bash replace-o1-image.sh NEW_IMAGE EXPECTED_REVISION [NAME=VALUE ...]
set -euo pipefail

new_image="${1:?new image reference}"
expected_revision="${2:?expected org.opencontainers.image.revision}"
shift 2
extra=("$@")

c=creg-phase-c-r7-o1
api=http://127.0.0.1:18080
stamp=$(date -u +%Y%m%dT%H%M%SZ)
previous="$c-before-$stamp"

DOCKER=(docker)
docker info >/dev/null 2>&1 || DOCKER=(sudo docker)
say() { echo "[replace-o1] $*"; }
die() { echo "[replace-o1] ABORT: $*" >&2; exit 1; }
json_field() { python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get(sys.argv[1], ""))' "$1"; }

# ── Preconditions (nothing changes yet) ─────────────────────────────────────
[[ "$("${DOCKER[@]}" inspect -f '{{.State.Running}}' "$c" 2>/dev/null)" == true ]] || die "$c is not running"
override_names=()
for kv in ${extra[@]+"${extra[@]}"}; do
  [[ "$kv" =~ ^[A-Z][A-Z0-9_]*= ]] || die "setting '$kv' is not NAME=VALUE"
  override_names+=("${kv%%=*}")
done

say "pulling $new_image"
"${DOCKER[@]}" pull -q "$new_image" >/dev/null || die "could not pull $new_image"
revision=$("${DOCKER[@]}" image inspect -f '{{index .Config.Labels "org.opencontainers.image.revision"}}' "$new_image")
[[ "$revision" == "$expected_revision" ]] || die "image revision is '$revision', expected $expected_revision"
"${DOCKER[@]}" run --rm --entrypoint test "$new_image" -x /app/creg-node || die "$new_image has no /app/creg-node"

old_image=$("${DOCKER[@]}" inspect -f '{{.Config.Image}}' "$c")
user=$("${DOCKER[@]}" inspect -f '{{.Config.User}}' "$c")
restart=$("${DOCKER[@]}" inspect -f '{{.HostConfig.RestartPolicy.Name}}' "$c")
[[ -n "$restart" && "$restart" != "no" ]] || restart=unless-stopped

env_args=()
while IFS= read -r line; do
  [[ -z "$line" ]] && continue
  name="${line%%=*}"
  skip=0
  for o in ${override_names[@]+"${override_names[@]}"}; do [[ "$name" == "$o" ]] && skip=1; done
  [[ $skip -eq 0 ]] && env_args+=(-e "$line")
done < <("${DOCKER[@]}" inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$c")
for kv in ${extra[@]+"${extra[@]}"}; do env_args+=(-e "$kv"); done

port_args=()
while IFS='|' read -r port host_ip host_port; do
  [[ -z "$port" ]] && continue
  if [[ -n "$host_ip" ]]; then port_args+=(-p "$host_ip:$host_port:$port"); else port_args+=(-p "$host_port:$port"); fi
done < <("${DOCKER[@]}" inspect -f '{{range $p, $conf := .HostConfig.PortBindings}}{{range $conf}}{{$p}}|{{.HostIp}}|{{.HostPort}}{{println}}{{end}}{{end}}' "$c")

mount_args=()
while IFS='|' read -r type source destination rw name; do
  [[ -z "$type" ]] && continue
  suffix=""
  [[ "$rw" == "false" ]] && suffix=":ro"
  case "$type" in
    bind) mount_args+=(-v "$source:$destination$suffix") ;;
    volume) mount_args+=(-v "$name:$destination$suffix") ;;
    *) die "unsupported mount type $type" ;;
  esac
done < <("${DOCKER[@]}" inspect -f '{{range .Mounts}}{{.Type}}|{{.Source}}|{{.Destination}}|{{.RW}}|{{.Name}}{{println}}{{end}}' "$c")

mapfile -t networks < <("${DOCKER[@]}" inspect -f '{{range $n, $v := .NetworkSettings.Networks}}{{println $n}}{{end}}' "$c" | sed '/^$/d')
[[ ${#networks[@]} -gt 0 ]] || die "could not read $c's networks"

before_stats=$(curl -fsS -m 10 "$api/v1/public/chain/stats") || die "$c's API is not answering before the swap"
before_tip=$(printf '%s' "$before_stats" | json_field tip_height)
chain_id=$(printf '%s' "$before_stats" | json_field chain_id)
say "old image $old_image; tip $before_tip on $chain_id"
say "user $user, restart $restart, ports ${port_args[*]}, networks ${networks[*]}, mounts ${#mount_args[@]}, settings ${#env_args[@]} (+ ${override_names[*]:-none})"

restore() {
  say "restoring the previous $c"
  "${DOCKER[@]}" stop --time 30 "$c" >/dev/null 2>&1 || true
  "${DOCKER[@]}" logs --tail 40 "$c" 2>&1 | sed 's/^/[new o1] /' || true
  "${DOCKER[@]}" rename "$c" "$c-failed-$stamp" 2>/dev/null || true
  "${DOCKER[@]}" rename "$previous" "$c"
  "${DOCKER[@]}" update --restart "$restart" "$c" >/dev/null
  "${DOCKER[@]}" start "$c" >/dev/null
}

# ── Swap ─────────────────────────────────────────────────────────────────────
say "stopping $c (kept as $previous)"
"${DOCKER[@]}" stop --time 30 "$c" >/dev/null
"${DOCKER[@]}" rename "$c" "$previous"
"${DOCKER[@]}" update --restart no "$previous" >/dev/null

run_args=(-d --name "$c" --restart "$restart" --network "${networks[0]}" --network-alias "$c")
[[ -n "$user" ]] && run_args+=(--user "$user")
if ! "${DOCKER[@]}" run "${run_args[@]}" "${port_args[@]}" "${mount_args[@]}" "${env_args[@]}" "$new_image" >/dev/null; then
  "${DOCKER[@]}" rm -f "$c" >/dev/null 2>&1 || true
  "${DOCKER[@]}" rename "$previous" "$c"
  "${DOCKER[@]}" update --restart "$restart" "$c" >/dev/null
  "${DOCKER[@]}" start "$c" >/dev/null
  die "the new $c did not start; previous container restored"
fi
for network in "${networks[@]:1}"; do
  "${DOCKER[@]}" network connect --alias "$c" "$network" "$c"
done

# ── Checks: same chain, tip not behind, verified spec, new RPC behaviour ─────
ok=0
for _ in $(seq 1 60); do
  stats=$(curl -fsS -m 5 "$api/v1/public/chain/stats" 2>/dev/null || true)
  if [[ -n "$stats" ]]; then
    tip=$(printf '%s' "$stats" | json_field tip_height)
    id=$(printf '%s' "$stats" | json_field chain_id)
    verified=$(printf '%s' "$stats" | json_field spec_signature_verified)
    if [[ "$id" == "$chain_id" && "$verified" == "True" && -n "$tip" && "$tip" -ge "$before_tip" ]]; then ok=1; break; fi
  fi
  sleep 3
done
if [[ $ok -ne 1 ]]; then
  restore
  die "the new $c did not reach chain $chain_id at tip >= $before_tip with a verified spec within 180 s"
fi
health=$(curl -fsS -m 10 -X POST "$api/rpc" -H 'content-type: application/json' --data-binary '{"jsonrpc":"2.0","method":"creg_health","id":1}' || true)
batch=$(curl -fsS -m 10 -X POST "$api/rpc" -H 'content-type: application/json' --data-binary '[{"jsonrpc":"2.0","method":"creg_chainId","id":1},{"jsonrpc":"2.0","method":"creg_blockNumber","id":2}]' || true)
if [[ "$health" != *tip_age_secs* || "$batch" != \[* ]]; then
  restore
  die "the new $c does not show the Phase 2 RPC behaviour (creg_health tip_age_secs, batch array)"
fi
trusted=$("${DOCKER[@]}" inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$c" | grep -E '^CREG_TRUSTED_PROXIES=' | cut -d= -f2- || true)
say "new $c up: tip $tip on $id, spec verified, creg_health has tip_age_secs, batches answered, CREG_TRUSTED_PROXIES=${trusted:-<unset>}"
say "REPLACE-OK stamp=$stamp (rollback: bash rollback-o1-image.sh $stamp)"
