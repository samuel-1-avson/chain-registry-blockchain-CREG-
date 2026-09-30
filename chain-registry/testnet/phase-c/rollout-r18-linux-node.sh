#!/usr/bin/env bash
# Guarded real-node replacement at the accepted h5 baseline. The journal is
# never copied, removed or edited; diagnostics use a stopped chain DB copy.
set -euo pipefail
slot=${1:?slot}; role=${2:?role}; api_port=${3:?API port}; p2p_port=${4:?P2P port}
new_image_id=${5:?pinned R18 image ID}
case "$slot:$role" in v[2-4]:validator|o[1-4]:observer) ;; *) exit 2 ;; esac
case "$api_port:$p2p_port" in 8080:9000|18080:9001) ;; *) exit 2 ;; esac
[[ "$new_image_id" =~ ^sha256:[a-f0-9]{64}$ ]]
image=creg-node:phase-c-secure-r18-canonical-round-recovery
container=creg-phase-c-r7-$slot
preserved=$container-r12-osv-r9-preserved-for-r18
base=/var/lib/creg/phase-c-r7
data_dir=$base/data
old_scanner=$base/scanner-r10-osv-r9
new_scanner=$base/scanner-r18-osv-r10
env_path=$base/node.env
env_backup=$base/node.env.r12-osv-r9-preserved-for-r18
snapshot_dir=$base/diagnostics-r18-h5
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
printf '%s  %s\n' "$new_osv_sha" /tmp/creg-osv-snapshot-r18.json | sha256sum -c - >/dev/null
printf '%s  %s\n' c5901bfe95fb6bebb8285202bddb0604514f73739443669215e044a7ae66deaa "$old_scanner/osv_snapshot.json" | sudo sha256sum -c - >/dev/null
printf '%s  %s\n' 5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef "$old_scanner/known_malicious_hashes.json" | sudo sha256sum -c - >/dev/null
printf '%s  %s\n' 217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e "$old_scanner/rules/supply_chain.yar" | sudo sha256sum -c - >/dev/null

check_live() {
    python3 - "$api" "$1" "$role" <<'PY'
import hashlib, json, re, sys, urllib.request
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
api, profile, role = sys.argv[1:]
def read(path, raw=False):
    with urllib.request.urlopen(api+path,timeout=8) as response: text=response.read().decode()
    return text if raw else json.loads(text)
s=read('/v1/public/chain/stats'); h=read('/v1/public/health')
m=read('/metrics',True); p=read('/v1/consensus/pbft')
with open('/tmp/creg-r7-signed-spec-r18.json','rb') as f: spec_bytes=f.read()
if hashlib.sha256(spec_bytes).hexdigest()!='331897f5a3f89fe902e4a7d814a9245ef50b384e22cc3d8dc946b4b45ed39e9f':
    raise SystemExit('Authenticated R7 spec pin mismatch')
spec=json.loads(spec_bytes)
roster={v['id']:v['pubkey'] for v in spec['validator_set']['validators']}
assert len(roster)==4 and spec['chain_id']==s.get('chain_id')
block=read('/v1/public/blocks/5')
expected_hash='daab438828a678aee3aa2eebf041f5147bc6f76fed837a207e6a1f4747d427cf'
assert block.get('hash')==expected_hash and block.get('finalized') is True
header=dict(block['header']); header.pop('view_change_certificate',None)
if header.get('view_number',0)==0: header.pop('view_number',None)
assert hashlib.sha256(json.dumps(header,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()==expected_hash
leaves=[hashlib.sha256(json.dumps(tx,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest() for tx in block['transactions']]
if not leaves: leaves=[hashlib.sha256(b'empty').hexdigest()]
while len(leaves)>1:
    if len(leaves)%2: leaves.append(leaves[-1])
    leaves=[hashlib.sha256((leaves[i]+leaves[i+1]).encode()).hexdigest() for i in range(0,len(leaves),2)]
assert leaves[0]==header['merkle_root']
signers=set()
for signature in block['pbft_signatures']:
    vid=signature['validator_id']; pubkey=roster.get(vid)
    assert pubkey==signature['pubkey'] and vid not in signers
    Ed25519PublicKey.from_public_bytes(bytes.fromhex(pubkey)).verify(bytes.fromhex(signature['signature']),('creg-pbft-v1:commit:'+expected_hash).encode())
    signers.add(vid)
assert len(signers)>=3
# A failed round with a PREPARE quorum also remains recovery-sensitive.
retained=[r for r in p.get('rounds',[]) if r.get('phase')!='finalised' and
          (r.get('phase')!='failed' or r.get('prepare_count',0)>=r.get('quorum',3))]
pbft_safe=not retained
if profile=='dd52b5f77776aa0cad9cb25a4a308f9ef3755c1d3addec44073a5a322ee2e4fb':
    pbft_safe=all(r.get('block_height')==5 and r.get('block_hash')==expected_hash for r in retained)
zero=lambda name: bool(re.search(r'^'+name+r' 0\s*$',m,re.M))
checks={
 'same_h5': s.get('tip_height')==5 and s.get('tip_hash')=='daab438828a678aee3aa2eebf041f5147bc6f76fed837a207e6a1f4747d427cf',
 'chain_identity': s.get('chain_id')=='creg-phase-c-testnet-2026-09-26-r7' and s.get('spec_signature_verified') is True,
 'four_validators': s.get('validator_count')==4 and s.get('active_validators')==4,
 'scanner_ready': bool(re.search(r'^creg_scanner_profile_ready 1\s*$',m,re.M)) and f'profile_digest="{profile}"' in m,
 'queues_clear': all(zero(n) for n in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')),
 'retained_rounds_safe': pbft_safe,
 'canonical_commit_certificate': len(signers)>=3,
 'peers_connected': s.get('peer_count',0)>0,
 'correct_role': h.get('is_validator')==(role=='validator'),
}
if role=='validator':
    sandbox=h.get('sandbox',{})
    checks['real_isolated_sandbox']=sandbox.get('engine')=='nsjail' and sandbox.get('isolated') is True and sandbox.get('dev_bypass') is False
print(json.dumps({'checks':checks,'passed':all(checks.values()),'stats':s,'health':h,'pbft':p,'retained_rounds':retained,'canonical_commit_signers':sorted(signers)}))
if not all(checks.values()): raise SystemExit(1)
PY
}
if ! check_live "$old_profile" > /tmp/creg-r18-$slot-preflight.json; then
    cat /tmp/creg-r18-$slot-preflight.json >&2
    echo 'Read-only preflight failed; no node was stopped or replaced' >&2; exit 1
fi
identity_before=$(sudo sha256sum "$identity" | cut -d ' ' -f1)
sudo cp -a "$old_scanner" "$new_scanner"
sudo install -m 0644 /tmp/creg-osv-snapshot-r18.json "$new_scanner/osv_snapshot.json"
sudo cp -p "$env_path" "$env_backup"
sudo docker stop --time 20 "$container" >/dev/null
[[ $(sudo docker inspect "$container" --format '{{.State.Status}}') == exited ]]
sudo mkdir -m 0700 "$snapshot_dir"
sudo cp -a "$data_dir/chain.rocksdb" "$snapshot_dir/chain.rocksdb"
# Use a read-only copy. The live DB and its durable PBFT journal stay mounted
# only by their node; no operator attempts to reopen them concurrently.
if ! sudo docker run --rm --user 0 --entrypoint /app/creg_recovery_inspect \
    --mount "type=bind,source=$snapshot_dir,target=/snapshot,readonly" \
    "$image" /snapshot > /tmp/creg-r18-$slot-queue-review.json; then
    sudo docker start "$container" >/dev/null
    echo 'Offline inspection failed; the prior node was restarted' >&2; exit 1
fi
sudo docker rename "$container" "$preserved"
sudo sed -i -e "s|^CREG_OSV_SNAPSHOT_EPOCH=.*|CREG_OSV_SNAPSHOT_EPOCH=$new_epoch|" \
    -e "s|^CREG_SCANNER_PROFILE_DIGEST=.*|CREG_SCANNER_PROFILE_DIGEST=$new_profile|" "$env_path"
args=(run -d --name "$container" --restart unless-stopped --env-file "$env_path"
 -p "$api_port:8080" -p "$p2p_port:9000"
 --mount "type=bind,source=$data_dir,target=/data"
 --mount "type=bind,source=$new_scanner,target=/scanner,readonly")
if [[ "$role" == validator ]]; then
    docker_group=$(stat -c '%g' /var/run/docker.sock)
    args+=(--privileged --group-add "$docker_group" -v /var/run/docker.sock:/var/run/docker.sock)
fi
sudo docker "${args[@]}" "$new_image_id" >/dev/null
[[ $(sudo docker inspect "$container" --format '{{.Image}}') == "$new_image_id" ]]
[[ $(sudo sha256sum "$identity" | cut -d ' ' -f1) == "$identity_before" ]]
deadline=$((SECONDS+360))
until check_live "$new_profile" > /tmp/creg-r18-$slot-rollout.json 2>/dev/null; do
    if (( SECONDS>=deadline )); then echo 'New node failed readiness; preserved prior container available' >&2; exit 1; fi
    sleep 2
done
if [[ "$role" == validator ]]; then sudo test -d "$data_dir/pbft-safety.rocksdb"; fi
[[ $(sudo docker inspect "$preserved" --format '{{.State.Status}}') == exited ]]
python3 - "$slot" "$old_image" "$new_image_id" "$preserved" <<'PY'
import json,sys,datetime
slot,old,new,preserved=sys.argv[1:]
with open('/tmp/creg-r18-'+slot+'-rollout.json') as f: result=json.load(f)
with open('/tmp/creg-r18-'+slot+'-queue-review.json') as f: queues=json.load(f)
with open('/tmp/creg-r18-'+slot+'-preflight.json') as f: before=json.load(f)
result.update(schema='creg-real-r18-rollout-v1',captured_at_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),slot=slot,prior_image=old,image_id=new,preserved_container=preserved,p2p_identity_preserved=True,signer_journal_untouched=True,queue_review=queues,before=before,public_cutover=False)
print(json.dumps(result))
PY
