#!/usr/bin/env bash
# Guarded real-node replacement at the accepted h5 baseline. The journal is
# never copied, removed or edited; diagnostics use a stopped chain DB copy.
set -euo pipefail
slot=${1:?slot}; role=${2:?role}; api_port=${3:?API port}; p2p_port=${4:?P2P port}
new_image_id=${5:?pinned R16 image ID}
case "$slot:$role" in v[2-4]:validator|o[1-4]:observer) ;; *) exit 2 ;; esac
case "$api_port:$p2p_port" in 8080:9000|18080:9001) ;; *) exit 2 ;; esac
[[ "$new_image_id" =~ ^sha256:[a-f0-9]{64}$ ]]
image=creg-node:phase-c-secure-r16-committed-events
container=creg-phase-c-r7-$slot
preserved=$container-r12-osv-r9-preserved-for-r16
base=/var/lib/creg/phase-c-r7
data_dir=$base/data
old_scanner=$base/scanner-r10-osv-r9
new_scanner=$base/scanner-r16-osv-r10
env_path=$base/node.env
env_backup=$base/node.env.r12-osv-r9-preserved-for-r16
snapshot_dir=$base/diagnostics-r16-h5
identity=$data_dir/p2p/identity.key
api=http://127.0.0.1:$api_port
old_profile=dd52b5f77776aa0cad9cb25a4a308f9ef3755c1d3addec44073a5a322ee2e4fb
new_profile=d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4
new_epoch=osv-2026-09-26-phase-c-r10-event-check
new_osv_sha=eda8108603d2d674fcdc385635f2e7ac3091a044322214d1c1568b79de8986b9
sudo test -f "$env_path"; sudo test -f "$identity"
sudo test -d "$data_dir/chain.rocksdb"; sudo test -d "$old_scanner"
if [[ "$role" == validator ]]; then sudo test -d "$data_dir/pbft-safety.rocksdb"; fi
for target in "$new_scanner" "$env_backup" "$snapshot_dir"; do
    if sudo test -e "$target"; then echo "Preservation target exists: $target" >&2; exit 1; fi
done
if sudo docker container inspect "$preserved" >/dev/null 2>&1; then exit 1; fi
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$old_profile" "$env_path"
sudo grep -Fxq 'CREG_OSV_SNAPSHOT_EPOCH=osv-2026-09-26-phase-c-r9-write-check' "$env_path"
sudo grep -Fxq 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$env_path"
sudo grep -Fxq "CREG_NODE_ROLE=$role" "$env_path"
[[ $(sudo docker inspect "$container" --format '{{.State.Status}}') == running ]]
old_image=$(sudo docker inspect "$container" --format '{{.Image}}')
case "$old_image" in
 sha256:e4790446f65de6e1404200bfe8d3a2e444cb80a997a16cdef969da2bfd62e2dd|sha256:f0b693dc567fc3430f22db881581c55a253e75c8f84a5a0f2d6a82d4460d0736) ;;
 *) echo 'Prior image is not the accepted R12 release' >&2; exit 1 ;;
esac
[[ $(sudo docker image inspect "$image" --format '{{.Id}}') == "$new_image_id" ]]
mounts=$(sudo docker inspect "$container" --format '{{range .Mounts}}{{println .Source .Destination}}{{end}}')
grep -Fxq "$data_dir /data" <<<"$mounts"
grep -Fxq "$old_scanner /scanner" <<<"$mounts"
printf '%s  %s\n' "$new_osv_sha" /tmp/creg-osv-snapshot-r16.json | sha256sum -c - >/dev/null
printf '%s  %s\n' c5901bfe95fb6bebb8285202bddb0604514f73739443669215e044a7ae66deaa "$old_scanner/osv_snapshot.json" | sudo sha256sum -c - >/dev/null
printf '%s  %s\n' 5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef "$old_scanner/known_malicious_hashes.json" | sudo sha256sum -c - >/dev/null
printf '%s  %s\n' 217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e "$old_scanner/rules/supply_chain.yar" | sudo sha256sum -c - >/dev/null

check_live() {
    python3 - "$api" "$1" "$role" <<'PY'
import json, re, sys, urllib.request
api, profile, role = sys.argv[1:]
def read(path, raw=False):
    with urllib.request.urlopen(api+path,timeout=8) as response: text=response.read().decode()
    return text if raw else json.loads(text)
s=read('/v1/public/chain/stats'); h=read('/v1/public/health')
m=read('/metrics',True); p=read('/v1/consensus/pbft')
zero=lambda name: bool(re.search(r'^'+name+r' 0\s*$',m,re.M))
checks={
 'same_h5': s.get('tip_height')==5 and s.get('tip_hash')=='daab438828a678aee3aa2eebf041f5147bc6f76fed837a207e6a1f4747d427cf',
 'chain_identity': s.get('chain_id')=='creg-phase-c-testnet-2026-09-26-r7' and s.get('spec_signature_verified') is True,
 'four_validators': s.get('validator_count')==4 and s.get('active_validators')==4,
 'scanner_ready': bool(re.search(r'^creg_scanner_profile_ready 1\s*$',m,re.M)) and f'profile_digest="{profile}"' in m,
 'queues_clear': all(zero(n) for n in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')),
 'no_active_pbft': all(r.get('phase') in ('finalised','failed') for r in p.get('rounds',[])),
 'peers_connected': s.get('peer_count',0)>0,
 'correct_role': h.get('is_validator')==(role=='validator'),
}
if role=='validator':
    sandbox=h.get('sandbox',{})
    checks['real_isolated_sandbox']=sandbox.get('engine')=='nsjail' and sandbox.get('isolated') is True and sandbox.get('dev_bypass') is False
print(json.dumps({'checks':checks,'passed':all(checks.values()),'stats':s,'health':h}))
if not all(checks.values()): raise SystemExit(1)
PY
}
check_live "$old_profile"
