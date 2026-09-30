#!/usr/bin/env bash
set -euo pipefail
slot=$1
role=$2
api_port=$3
p2p_port=$4
old_epoch=$5
old_profile=$6
new_epoch=$7
new_profile=$8
new_osv_sha=$9
case "$slot:$role" in v[1-4]:validator|o[1-4]:observer) ;; *) echo invalid slot role >&2; exit 2 ;; esac
case "$api_port:$p2p_port" in 8080:9000|18080:9001) ;; *) echo invalid ports >&2; exit 2 ;; esac
container="creg-phase-c-r7-$slot"
base=/var/lib/creg/phase-c-r7
data_dir="$base/data"
scanner_dir="$base/scanner"
new_scanner="$base/scanner-r10-osv-r9"
env_path="$base/node.env"
env_backup="$base/node.env.osv-r8-preserved"
identity_path="$data_dir/p2p/identity.key"
journal_path="$data_dir/pbft-safety.rocksdb"
snapshot_src=/tmp/creg-osv-snapshot-r9.json
image=creg-node:phase-c-secure-r12-bounded-l1-retry
config_digest=sha256:e4790446f65de6e1404200bfe8d3a2e444cb80a997a16cdef969da2bfd62e2dd
index_digest=sha256:f0b693dc567fc3430f22db881581c55a253e75c8f84a5a0f2d6a82d4460d0736
preserved="$container-r12-osv-r8-preserved"
api="http://127.0.0.1:$api_port"
sudo test -f "$env_path"
sudo test -f "$identity_path"
sudo test -f "$snapshot_src"
sudo test -d "$data_dir"
sudo test -d "$scanner_dir"
if [[ "$role" == validator ]]; then sudo test -d "$journal_path"; fi
if sudo test -e "$new_scanner" || sudo test -e "$env_backup"; then echo "profile backup already exists" >&2; exit 1; fi
sudo grep -Fxq "CREG_OSV_SNAPSHOT_EPOCH=$old_epoch" "$env_path"
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$old_profile" "$env_path"
printf '%s  %s\n' "$new_osv_sha" "$snapshot_src" | sha256sum -c - >/dev/null
printf '%s  %s\n' 5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef "$scanner_dir/known_malicious_hashes.json" | sudo sha256sum -c - >/dev/null
printf '%s  %s\n' 217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e "$scanner_dir/rules/supply_chain.yar" | sudo sha256sum -c - >/dev/null
image_id=$(sudo docker image inspect "$image" --format '{{.Id}}')
[[ "$image_id" == "$config_digest" || "$image_id" == "$index_digest" ]]
[[ $(sudo docker inspect "$container" --format '{{.State.Status}}') == running ]]
if sudo docker container inspect "$preserved" >/dev/null 2>&1; then echo "container preservation target exists" >&2; exit 1; fi
mounts=$(sudo docker inspect "$container" --format '{{range .Mounts}}{{println .Source .Destination}}{{end}}')
grep -Fxq "$data_dir /data" <<<"$mounts"
grep -Fxq "$scanner_dir /scanner" <<<"$mounts"
curl -fsS --max-time 8 "$api/v1/public/chain/stats" | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s.get("chain_id")=="creg-phase-c-testnet-2026-09-26-r7" and s.get("tip_height")==4 and s.get("tip_hash")=="0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0", "unexpected chain tip"'
curl -fsS --max-time 8 "$api/v1/consensus/pbft" | python3 -c 'import json,sys; assert all(r.get("phase") in ("finalised","failed") for r in json.load(sys.stdin).get("rounds",[])), "active PBFT round"'
identity_before=$(sudo sha256sum "$identity_path" | cut -d ' ' -f1)
sudo cp -a "$scanner_dir" "$new_scanner"
sudo install -m 0644 "$snapshot_src" "$new_scanner/osv_snapshot.json"
printf '%s  %s\n' "$new_osv_sha" "$new_scanner/osv_snapshot.json" | sudo sha256sum -c - >/dev/null
sudo cp -p "$env_path" "$env_backup"
sudo sed -i -e "s|^CREG_OSV_SNAPSHOT_EPOCH=.*|CREG_OSV_SNAPSHOT_EPOCH=$new_epoch|" -e "s|^CREG_SCANNER_PROFILE_DIGEST=.*|CREG_SCANNER_PROFILE_DIGEST=$new_profile|" "$env_path"
sudo grep -Fxq "CREG_OSV_SNAPSHOT_EPOCH=$new_epoch" "$env_path"
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$new_profile" "$env_path"
sudo docker stop --time 20 "$container" >/dev/null
[[ $(sudo docker inspect "$container" --format '{{.State.Status}}') == exited ]]
sudo docker rename "$container" "$preserved"
args=(run -d --name "$container" --restart unless-stopped --env-file "$env_path" -p "$api_port:8080" -p "$p2p_port:9000" --mount "type=bind,source=$data_dir,target=/data" --mount "type=bind,source=$new_scanner,target=/scanner,readonly")
if [[ "$role" == validator ]]; then docker_group=$(stat -c '%g' /var/run/docker.sock); args+=(--privileged --group-add "$docker_group" -v /var/run/docker.sock:/var/run/docker.sock); fi
sudo docker "${args[@]}" "$image" >/dev/null
actual_image=$(sudo docker inspect "$container" --format '{{.Image}}')
[[ "$actual_image" == "$config_digest" || "$actual_image" == "$index_digest" ]]
[[ $(sudo sha256sum "$identity_path" | cut -d ' ' -f1) == "$identity_before" ]]
deadline=$((SECONDS+360))
while true; do
    if curl -fsS --max-time 3 "$api/v1/public/health" >/dev/null 2>&1; then
        metrics=$(curl -fsS --max-time 8 "$api/metrics" 2>/dev/null || true)
        stats=$(curl -fsS --max-time 8 "$api/v1/public/chain/stats" 2>/dev/null || true)
        pbft=$(curl -fsS --max-time 8 "$api/v1/consensus/pbft" 2>/dev/null || true)
        if [[ -n "$metrics" && -n "$stats" && -n "$pbft" ]] &&
           python3 - "$slot" "$role" "$new_profile" "$metrics" "$stats" "$pbft" >/dev/null 2>&1 <<'PY'
import json,sys
slot,role,profile,metrics_text,stats_text,pbft_text=sys.argv[1:]
s=json.loads(stats_text); rounds=json.loads(pbft_text).get('rounds',[])
ok=(s.get('chain_id')=='creg-phase-c-testnet-2026-09-26-r7' and s.get('tip_height')==4 and
    s.get('tip_hash')=='0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0' and
    s.get('peer_count',0)>0 and 'creg_scanner_profile_ready 1' in metrics_text and
    f'profile_digest="{profile}"' in metrics_text and
    all(f'{name} 0' in metrics_text for name in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')) and
    all(r.get('phase') in ('finalised','failed') for r in rounds))
if role=='validator': ok=ok and s.get('validator_count')==4 and s.get('active_validators')==4
if not ok: raise SystemExit(1)
PY
        then break; fi
    fi
    if (( SECONDS >= deadline )); then echo "node failed scanner-profile readiness" >&2; exit 1; fi
    sleep 2
done
metrics=$(curl -fsS --max-time 8 "$api/metrics")
stats=$(curl -fsS --max-time 8 "$api/v1/public/chain/stats")
pbft=$(curl -fsS --max-time 8 "$api/v1/consensus/pbft")
python3 - "$slot" "$role" "$new_profile" "$metrics" "$stats" "$pbft" <<'PY'
import json,sys
slot,role,profile,metrics_text,stats_text,pbft_text=sys.argv[1:]
s=json.loads(stats_text); rounds=json.loads(pbft_text).get('rounds',[])
checks={
 'same_h4': s.get('chain_id')=='creg-phase-c-testnet-2026-09-26-r7' and s.get('tip_height')==4 and s.get('tip_hash')=='0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0',
 'scanner_ready': 'creg_scanner_profile_ready 1' in metrics_text and f'profile_digest="{profile}"' in metrics_text,
 'queue_clear': all(f'{name} 0' in metrics_text for name in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')),
 'no_active_pbft': all(r.get('phase') in ('finalised','failed') for r in rounds),
 'live_peers': s.get('peer_count',0)>0,
}
if role=='validator': checks['four_validators']=s.get('validator_count')==4 and s.get('active_validators')==4
print(json.dumps({'slot':slot,'checks':checks,'passed':all(checks.values())}))
if not all(checks.values()): raise SystemExit('new scanner profile failed live checks')
PY
if [[ "$role" == validator ]]; then sudo test -d "$journal_path"; fi
echo "scanner profile updated on $slot; prior=$preserved; data and identity preserved"
