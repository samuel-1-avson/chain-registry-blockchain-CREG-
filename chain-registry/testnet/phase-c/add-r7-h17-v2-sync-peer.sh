#!/usr/bin/env bash
# Add the real V2 h17 API as an authenticated sync source for one R7 node.
# Each invocation preserves the node's data, identity, journal, env and container.
set -euo pipefail
slot=${1:?one of v4 o1 o2 o3 o4}
base=/var/lib/creg/phase-c-r7
data=$base/data
env_file=$base/node.env
scanner=$base/scanner-r13-osv-has-flag-live
profile=8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e
height_hash=7c3364858873b00dcb8eef4875c2f1eaa85ade32648850177bd12dcb5f7627d7
case "$slot" in
  v4)
    role=validator; node_id=validator-4; api_port=8080; p2p_port=9000
    image=sha256:704cd7dfb89a048c146a3de502416c31a6f77c0e3f493a98f4fe3098c732046c
    old_peers=CREG_PEERS=http://10.96.1.2:8080,http://10.96.4.2:8080
    source_url=http://34.74.35.254:18080 ;;
  o1)
    role=observer; node_id=observer-1; api_port=18080; p2p_port=9001
    image=sha256:ef21b958b5e3e4f09e1c87290d3d9a50f23b5350d207e5af9982dd1cf5413708
    old_peers=CREG_PEERS=http://34.156.122.19:8080,http://34.124.165.97:8080
    source_url=http://10.142.0.4:18080 ;;
  o2)
    role=observer; node_id=observer-2; api_port=8080; p2p_port=9000
    image=sha256:2a553f1cb8b2ea6c63bf99649c57e0291698e3ac01da5de80863760afcc07a70
    old_peers=CREG_PEERS=http://34.156.122.19:8080,http://34.124.165.97:8080
    source_url=http://10.142.0.4:18080 ;;
  o3|o4)
    role=observer; node_id=observer-${slot#o}; api_port=8080; p2p_port=9000
    image=sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9
    old_peers=CREG_PEERS=http://10.96.1.2:8080,http://10.96.2.2:8080
    source_url=http://34.74.35.254:18080 ;;
  *) echo 'Unsupported R7 slot' >&2; exit 2 ;;
esac
new_peers=CREG_PEERS=$source_url,${old_peers#CREG_PEERS=}
container=creg-phase-c-r7-$slot
run_id=$(date -u +%Y%m%dT%H%M%SZ)
preserved=$container-before-v2-sync-peer-$run_id
env_backup=$base/node.env.before-v2-sync-peer-$run_id
evidence_dir=$base/evidence
preflight=$evidence_dir/v2-sync-peer-$slot-preflight-$run_id.json
postflight=$evidence_dir/v2-sync-peer-$slot-postflight-$run_id.json

exec 9>/tmp/creg-r7-v2-sync-peer-$slot.lock
flock -n 9 || { echo 'Node peer update already running' >&2; exit 1; }
[[ ! -e $env_backup && ! -e $preflight && ! -e $postflight ]]
[[ -d $data/chain.rocksdb && -f $data/p2p/identity.key && -d $scanner && -f $env_file ]]
if [[ $role == validator ]]; then [[ -d $data/pbft-safety.rocksdb ]]; fi
grep -Fxq "CREG_NODE_ID=$node_id" "$env_file"
grep -Fxq "CREG_NODE_ROLE=$role" "$env_file"
grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$profile" "$env_file"
grep -Fxq "$old_peers" "$env_file"
[[ $(docker inspect "$container" --format '{{.Image}} {{.State.Status}}') == "$image running" ]]
docker inspect "$preserved" >/dev/null 2>&1 && { echo 'Preserved container name exists' >&2; exit 1; }
install -d -m 0755 "$evidence_dir"

python3 - "$slot" "$role" "$api_port" "$node_id" "$image" "$data" "$scanner" "$source_url" "$height_hash" "$profile" "$container" >"$preflight" <<'PY'
import json,re,subprocess,sys,urllib.request
slot,role,port,node_id,image,data,scanner,source,hash17,profile,container=sys.argv[1:]
def get(base,path):
 with urllib.request.urlopen(base+path,timeout=10) as response:return json.load(response)
local='http://127.0.0.1:'+port
stats=get(local,'/v1/public/chain/stats')
health=get(local,'/v1/public/health')
pbft=get(local,'/v1/consensus/pbft')
source_stats=get(source,'/v1/public/chain/stats')
source_block=get(source,'/v1/public/blocks/17')
with urllib.request.urlopen(local+'/metrics',timeout=10) as response:metrics=response.read().decode()
info=json.loads(subprocess.check_output(['docker','inspect',container]))[0]
mounts={(m['Source'],m['Destination'],m['RW']) for m in info['Mounts']}
def metric(name):
 match=re.search(r'^'+re.escape(name)+r'\s+([-+]?[0-9.]+)$',metrics,re.M)
 return float(match.group(1)) if match else None
unresolved=[r for r in pbft['rounds'] if r['phase']!='finalised' and (r['phase']!='failed' or r['prepare_count']>=r['quorum'])]
checks={
 'current_signed_r7_h17':stats['chain_id']=='creg-phase-c-testnet-2026-09-26-r7' and stats['spec_signature_verified'] is True and stats['tip_height']==stats['finalized_height']==17 and stats['tip_hash']==hash17,
 'real_v2_h17_source':source_stats['chain_id']==stats['chain_id'] and source_stats['spec_signature_verified'] is True and source_stats['tip_height']==source_stats['finalized_height']==17 and source_stats['tip_hash']==hash17 and source_block['hash']==hash17 and source_block['finalized'] is True and len({s['validator_id'] for s in source_block['pbft_signatures']})>=3,
 'identity_and_mounts':health['node_id']==node_id and health['is_validator'] is (role=='validator') and info['Image']==image and (data,'/data',True) in mounts and (scanner,'/scanner',False) in mounts,
 'r13_scanner':f'profile_digest="{profile}"' in metrics and metric('creg_scanner_profile_ready')==1,
 'queues_and_pbft_clear':not unresolved and all(metric(name)==0 for name in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_forced_inclusion_pending')),
}
attached_networks=sorted(set(info['NetworkSettings']['Networks'])-{'bridge'})
print(json.dumps({'schema':'creg-r7-v2-sync-peer-preflight-v1','slot':slot,'checks':checks,'height':stats['tip_height'],'block_hash':hash17,'dead_letters':metric('creg_durable_finalized_work_dead_letter_depth'),'attached_networks':attached_networks,'passed':all(checks.values())}))
if not all(checks.values()):raise SystemExit('V2 sync-peer preflight failed')
PY
identity_before=$(sha256sum "$data/p2p/identity.key" | cut -d' ' -f1)
dead_before=$(python3 - "$preflight" <<'PY'
import json,sys
print(json.load(open(sys.argv[1]))['dead_letters'])
PY
)
cp -p "$env_file" "$env_backup"
python3 - "$env_file" "$old_peers" "$new_peers" <<'PY'
import os,pathlib,sys,tempfile
path,old,new=sys.argv[1:]
p=pathlib.Path(path)
body=p.read_bytes().decode()
if body.splitlines().count(old)!=1:raise SystemExit('Expected exactly one old peer setting')
body=body.replace(old,new,1)
fd,tmp=tempfile.mkstemp(prefix='.v2-sync-peer-',dir=str(p.parent))
try:
 os.fchmod(fd,0o600)
 with os.fdopen(fd,'wb') as out:out.write(body.encode());out.flush();os.fsync(out.fileno())
 os.replace(tmp,p)
finally:
 if os.path.exists(tmp):os.unlink(tmp)
PY
grep -Fxq "$new_peers" "$env_file"
docker stop --time 30 "$container" >/dev/null
docker rename "$container" "$preserved"
args=(run -d --name "$container" --restart unless-stopped --env-file "$env_file"
  -p "$api_port:8080" -p "$p2p_port:9000"
  --mount "type=bind,source=$data,target=/data"
  --mount "type=bind,source=$scanner,target=/scanner,readonly")
if [[ $role == validator ]]; then
 docker_group=$(stat -c '%g' /var/run/docker.sock)
 args+=(--privileged --group-add "$docker_group" --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock)
fi
docker "${args[@]}" "$image" >/dev/null

# docker run replaces the container on the default bridge. Restore every
# additional network before checking the node; O1 needs the Prometheus and
# private explorer networks for its real history and candidate API routes.
mapfile -t attached_networks < <(python3 - "$preflight" <<'PY'
import json,sys
for network in json.load(open(sys.argv[1]))['attached_networks']:
 print(network)
PY
)
for network in "${attached_networks[@]}"; do
 docker network connect --alias "$container" "$network" "$container"
done

deadline=$((SECONDS+240))
while (( SECONDS < deadline )); do
 if python3 - "$slot" "$role" "$api_port" "$node_id" "$image" "$data" "$scanner" "$height_hash" "$profile" "$container" "$preserved" "$identity_before" "$dead_before" "$preflight" >"$postflight" 2>/tmp/creg-r7-v2-sync-peer-$slot-error.log <<'PY'
import hashlib,json,os,re,subprocess,sys,urllib.request
slot,role,port,node_id,image,data,scanner,hash17,profile,container,preserved,identity,dead_before,preflight=sys.argv[1:]
def get(path):
 with urllib.request.urlopen('http://127.0.0.1:'+port+path,timeout=8) as response:return json.load(response)
def metric(text,name):
 match=re.search(r'^'+re.escape(name)+r'\s+([-+]?[0-9.]+)$',text,re.M)
 return float(match.group(1)) if match else None
info=json.loads(subprocess.check_output(['docker','inspect',container]))[0]
prior=json.loads(subprocess.check_output(['docker','inspect',preserved]))[0]
expected_networks=set(json.load(open(preflight))['attached_networks'])
stats=get('/v1/public/chain/stats')
health=get('/v1/public/health')
pbft=get('/v1/consensus/pbft')
block=get('/v1/public/blocks/17')
with urllib.request.urlopen('http://127.0.0.1:'+port+'/metrics',timeout=8) as response:metrics=response.read().decode()
mounts={(m['Source'],m['Destination'],m['RW']) for m in info['Mounts']}
unresolved=[r for r in pbft['rounds'] if r['phase']!='finalised' and (r['phase']!='failed' or r['prepare_count']>=r['quorum'])]
checks={
 'same_image_and_healthy':info['Image']==image and info['State']['Running'] and info['State']['Health']['Status']=='healthy',
 'old_container_preserved':prior['State']['Status']=='exited',
 'same_data_scanner_identity':(data,'/data',True) in mounts and (scanner,'/scanner',False) in mounts and hashlib.sha256(open(data+'/p2p/identity.key','rb').read()).hexdigest()==identity,
 'same_signing_journal':role!='validator' or os.path.isdir(data+'/pbft-safety.rocksdb'),
 'signed_r7_h17':stats['chain_id']=='creg-phase-c-testnet-2026-09-26-r7' and stats['spec_signature_verified'] is True and stats['tip_height']==stats['finalized_height']==17 and stats['tip_hash']==hash17 and block['hash']==hash17 and block['finalized'] is True,
 'role_peers_and_scanner':health['node_id']==node_id and health['is_validator'] is (role=='validator') and stats['peer_count']>0 and f'profile_digest="{profile}"' in metrics and metric(metrics,'creg_scanner_profile_ready')==1,
 'active_queues_clear':all(metric(metrics,name)==0 for name in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_forced_inclusion_pending')),
 'pbft_clear':not unresolved,
 'dead_letters_preserved':metric(metrics,'creg_durable_finalized_work_dead_letter_depth')==float(dead_before),
 'network_attachments_preserved':expected_networks <= set(info['NetworkSettings']['Networks']),
}
if slot=='o1':
 history=get('/v1/public/metrics/history?range=30m')
 checks['real_metrics_history_ready']=history['status']=='ready' and history['chain_id']==stats['chain_id'] and history['sample_count']>0 and history['queried_series']==7
print(json.dumps({'schema':'creg-r7-v2-sync-peer-postflight-v1','slot':slot,'checks':checks,'height':stats['tip_height'],'block_hash':stats['tip_hash'],'passed':all(checks.values()),'preserved_container':preserved}))
if not all(checks.values()):raise SystemExit('V2 sync-peer postflight incomplete')
PY
 then cat "$postflight"; exit 0; fi
 sleep 4
done
echo 'Sync-peer postflight failed; data, journal, env backup and old container remain preserved.' >&2
cat /tmp/creg-r7-v2-sync-peer-$slot-error.log >&2
exit 1
