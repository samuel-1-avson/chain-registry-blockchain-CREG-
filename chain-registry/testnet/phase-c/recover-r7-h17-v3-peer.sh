#!/usr/bin/env bash
# Reconnect the real V3 validator to the V2 API holding the certified h17.
# Preserve V3's existing chain, identity, PBFT signing journal and old container.
set -euo pipefail

base=/var/lib/creg/phase-c-r7
data=$base/data
env_file=$base/node.env
scanner=$base/scanner-r13-osv-has-flag-live
container=creg-phase-c-r7-v3
image=sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9
old_peers=CREG_PEERS=http://10.96.2.2:8080,http://10.96.4.2:8080
new_peers=CREG_PEERS=http://34.74.35.254:18080,http://10.96.2.2:8080,http://10.96.4.2:8080
expected_h16=4bd594d71b4fc8bc785c34b1e46291518328233c8aed616171efd9add073e260
expected_h17=7c3364858873b00dcb8eef4875c2f1eaa85ade32648850177bd12dcb5f7627d7
profile=8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e
run_id=$(date -u +%Y%m%dT%H%M%SZ)
preserved=$container-before-h17-sync-$run_id
env_backup=$base/node.env.before-h17-sync-$run_id
evidence_dir=$base/evidence
preflight=$evidence_dir/h17-v3-preflight-$run_id.json
postflight=$evidence_dir/h17-v3-postflight-$run_id.json

exec 9>/tmp/creg-r7-h17-v3-recovery.lock
flock -n 9 || { echo 'V3 recovery is already in progress' >&2; exit 1; }
[[ ! -e $env_backup && ! -e $preflight && ! -e $postflight ]]
[[ -d $data/chain.rocksdb && -d $data/pbft-safety.rocksdb && -f $data/p2p/identity.key ]]
[[ -f $env_file && -d $scanner ]]
grep -Fxq 'CREG_NODE_ID=validator-3' "$env_file"
grep -Fxq 'CREG_NODE_ROLE=validator' "$env_file"
grep -Fxq "$old_peers" "$env_file"
grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$profile" "$env_file"
[[ $(docker inspect "$container" --format '{{.Image}} {{.State.Status}}') == "$image running" ]]
docker inspect "$preserved" >/dev/null 2>&1 && { echo 'Preserved container name already exists' >&2; exit 1; }
install -d -m 0755 "$evidence_dir"

python3 - "$expected_h16" "$expected_h17" "$profile" "$image" "$container" "$data" "$scanner" >"$preflight" <<'PY'
import hashlib,json,subprocess,sys,urllib.request
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
h16,h17,profile,image,container,data,scanner=sys.argv[1:]
def get(base,path):
 with urllib.request.urlopen(base+path,timeout=10) as response:return json.load(response)
local='http://127.0.0.1:8080'
source='http://34.74.35.254:18080'
stats=get(local,'/v1/public/chain/stats')
health=get(local,'/v1/public/health')
pbft=get(local,'/v1/consensus/pbft')
source_stats=get(source,'/v1/public/chain/stats')
block=get(source,'/v1/public/blocks/17')
with urllib.request.urlopen(local+'/metrics',timeout=10) as response:metrics=response.read().decode()
info=json.loads(subprocess.check_output(['docker','inspect',container]))[0]
mounts={(m['Source'],m['Destination'],m['RW']) for m in info['Mounts']}
header=dict(block['header'])
header.pop('view_change_certificate',None)
if header.get('view_number',0)==0:header.pop('view_number',None)
computed=hashlib.sha256(json.dumps(header,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
roster={
 'validator-1':'cc4157c4443c9fb011df2b650f497e3eadfc1ddda6dc7f4b6732325383108ffc',
 'validator-2':'779c9522addbce63f78448fa83bff1e789a4a874c4db25e0f7fe00b3d0429fd9',
 'validator-3':'7876b26b471f81edd9597354fd023157858f2b02d73a1e7bb0b430c9fc63e6bc',
 'validator-4':'f777ca6f1dafbbeb9cbe5a258568c574e7f519a1e58da57c97d28fcf69dd35ff'}
signers=set()
for signature in block['pbft_signatures']:
 vid=signature['validator_id']
 assert vid in roster and vid not in signers and signature['pubkey']==roster[vid]
 Ed25519PublicKey.from_public_bytes(bytes.fromhex(roster[vid])).verify(bytes.fromhex(signature['signature']),('creg-pbft-v1:commit:'+h17).encode())
 signers.add(vid)
published=any(tx.get('type')=='publish_authorized_v1' and tx.get('record',{}).get('id')=={'ecosystem':'npm','name':'has-flag','version':'5.0.1'} for tx in block['transactions'])
checks={
 'local_signed_r7_h16':stats['chain_id']=='creg-phase-c-testnet-2026-09-26-r7' and stats['spec_signature_verified'] is True and stats['tip_height']==stats['finalized_height']==16 and stats['tip_hash']==h16,
 'source_signed_r7_h17':source_stats['chain_id']==stats['chain_id'] and source_stats['spec_signature_verified'] is True and source_stats['tip_height']==source_stats['finalized_height']==17 and source_stats['tip_hash']==h17,
 'certified_same_parent':block['hash']==computed==h17 and block['header']['height']==17 and block['header']['prev_hash']==h16 and block['finalized'] is True and len(signers)>=3,
 'real_package_in_block':published,
 'identity_and_mounts':health['node_id']=='validator-3' and health['is_validator'] is True and info['Image']==image and (data,'/data',True) in mounts and (scanner,'/scanner',False) in mounts,
 'r13_scanner':f'profile_digest="{profile}"' in metrics and 'creg_scanner_profile_ready 1' in metrics,
 'no_conflicting_round':all(r.get('block_height')!=17 or r.get('block_hash')==h17 for r in pbft['rounds']),
}
print(json.dumps({'schema':'creg-r7-h17-v3-peer-recovery-preflight-v1','checks':checks,'local_height':stats['tip_height'],'source_height':source_stats['tip_height'],'source_block_hash':h17,'source_commit_signers':sorted(signers),'existing_rounds':pbft['rounds'],'passed':all(checks.values())}))
if not all(checks.values()):raise SystemExit('h17 recovery preflight rejected the current state')
PY

identity_before=$(sha256sum "$data/p2p/identity.key" | cut -d' ' -f1)
cp -p "$env_file" "$env_backup"
python3 - "$env_file" "$old_peers" "$new_peers" <<'PY'
import os,pathlib,sys,tempfile
path,old,new=sys.argv[1:]
p=pathlib.Path(path)
body=p.read_bytes().decode()
if body.splitlines().count(old)!=1:raise SystemExit('Expected exactly one old peer setting')
body=body.replace(old,new,1)
fd,tmp=tempfile.mkstemp(prefix='.h17-peers-',dir=str(p.parent))
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
docker_group=$(stat -c '%g' /var/run/docker.sock)
run_args=(run -d --name "$container" --restart unless-stopped --env-file "$env_file"
  -p 8080:8080 -p 9000:9000
  --mount "type=bind,source=$data,target=/data"
  --mount "type=bind,source=$scanner,target=/scanner,readonly"
  --privileged --group-add "$docker_group"
  --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock)
docker "${run_args[@]}" "$image" >/dev/null

deadline=$((SECONDS+300))
while (( SECONDS < deadline )); do
 if python3 - "$expected_h17" "$profile" "$image" "$container" "$preserved" "$data" "$scanner" "$identity_before" >"$postflight" 2>/tmp/creg-r7-h17-v3-postcheck-error.log <<'PY'
import hashlib,json,re,subprocess,sys,urllib.request
h17,profile,image,container,preserved,data,scanner,identity=sys.argv[1:]
def get(path):
 with urllib.request.urlopen('http://127.0.0.1:8080'+path,timeout=8) as response:return json.load(response)
def metric(text,name):
 match=re.search(r'^'+re.escape(name)+r'\s+([-+]?[0-9.]+)$',text,re.M)
 return float(match.group(1)) if match else None
info=json.loads(subprocess.check_output(['docker','inspect',container]))[0]
prior=json.loads(subprocess.check_output(['docker','inspect',preserved]))[0]
stats=get('/v1/public/chain/stats')
health=get('/v1/public/health')
pbft=get('/v1/consensus/pbft')
block=get('/v1/public/blocks/17')
package=get('/v1/public/packages/npm%3Ahas-flag%405.0.1')
with urllib.request.urlopen('http://127.0.0.1:8080/metrics',timeout=8) as response:metrics=response.read().decode()
mounts={(m['Source'],m['Destination'],m['RW']) for m in info['Mounts']}
unresolved=[r for r in pbft['rounds'] if r['phase']!='finalised' and (r['phase']!='failed' or r['prepare_count']>=r['quorum'])]
checks={
 'same_image_and_healthy':info['Image']==image and info['State']['Running'] and info['State']['Health']['Status']=='healthy',
 'old_container_preserved':prior['State']['Status']=='exited',
 'same_data_and_scanner_mounts':(data,'/data',True) in mounts and (scanner,'/scanner',False) in mounts,
 'same_identity_and_journal':hashlib.sha256(open(data+'/p2p/identity.key','rb').read()).hexdigest()==identity and __import__('os').path.isdir(data+'/pbft-safety.rocksdb'),
 'signed_r7_h17':stats['chain_id']=='creg-phase-c-testnet-2026-09-26-r7' and stats['spec_signature_verified'] is True and stats['tip_height']==stats['finalized_height']==17 and stats['tip_hash']==h17 and block['hash']==h17 and block['finalized'] is True,
 'real_package_verified':package.get('canonical')=='npm:has-flag@5.0.1' and package.get('status')=='verified' and package.get('block_hash')==h17,
 'role_peers_and_scanner':health['node_id']=='validator-3' and health['is_validator'] is True and stats['peer_count']>0 and f'profile_digest="{profile}"' in metrics and 'creg_scanner_profile_ready 1' in metrics,
 'active_queues_clear':all(metric(metrics,name)==0 for name in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_forced_inclusion_pending')),
 'pbft_clear':not unresolved,
}
print(json.dumps({'schema':'creg-r7-h17-v3-peer-recovery-postflight-v1','checks':checks,'height':stats['tip_height'],'block_hash':stats['tip_hash'],'package_status':package.get('status'),'dead_letter_depth':metric(metrics,'creg_durable_finalized_work_dead_letter_depth'),'passed':all(checks.values()),'preserved_container':preserved}))
if not all(checks.values()):raise SystemExit('h17 recovery postflight is incomplete')
PY
 then
  cat "$postflight"
  exit 0
 fi
 sleep 5
done
echo 'V3 recovery did not pass postflight. Existing data, signer journal, and prior container remain preserved.' >&2
cat /tmp/creg-r7-h17-v3-postcheck-error.log >&2
exit 1
