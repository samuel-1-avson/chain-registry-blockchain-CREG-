#!/usr/bin/env bash
# Replace one real R7 Linux node with the fresh R13 scanner bundle. Persistent
# chain data, identity, and validator signing journal stay on the host.
set -euo pipefail

slot=${1:?slot}; role=${2:?role}; api_port=${3:?API port}; p2p_port=${4:?P2P port}
node_id=${5:?node ID}; expected_image=${6:?current Docker image ID}
run_id=$(date -u +%Y%m%dT%H%M%SZ)
base=/var/lib/creg/phase-c-r7; data=$base/data; env_file=$base/node.env
old_scanner=$base/scanner-r12-osv-escape-live; scanner=$base/scanner-r13-osv-has-flag-live
stage=/tmp/creg-scanner-r13-$slot; container=creg-phase-c-r7-$slot
preserved=$container-r12-before-r13-$run_id
epoch=osv-2026-09-28-phase-c-has-flag-r13-live
old_epoch=osv-2026-09-27-phase-c-escape-string-regexp-r12-live
old_profile=e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2
profile=8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e
chain_id=creg-phase-c-testnet-2026-09-26-r7
genesis=0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4
tip_hash=4bd594d71b4fc8bc785c34b1e46291518328233c8aed616171efd9add073e260
capture_env=$base/node.env.r12-preserved-$run_id
evidence=$base/evidence/r13-$slot-rollout-$run_id.json

case "$slot:$role:$api_port:$p2p_port:$node_id" in
  v2:validator:18080:9001:validator-2|v3:validator:8080:9000:validator-3|v4:validator:8080:9000:validator-4|o1:observer:18080:9001:observer-1|o2:observer:8080:9000:observer-2|o3:observer:8080:9000:observer-3|o4:observer:8080:9000:observer-4) ;;
  *) echo 'Unexpected R7 node identity or port map' >&2; exit 2 ;;
esac
exec 9>/tmp/creg-r13-$slot-rollout.lock
flock -n 9 || { echo 'R13 rollout already active for this node' >&2; exit 1; }
[[ $(sudo docker inspect "$container" --format '{{.Image}} {{.State.Status}}') == "$expected_image running" ]]
[[ -d "$data/chain.rocksdb" && -f "$data/p2p/identity.key" && -f "$env_file" && -d "$old_scanner" ]]
[[ ! -e "$scanner" && ! -e "$capture_env" && ! -e "$evidence" ]]
sudo grep -Fxq "CREG_NODE_ID=$node_id" "$env_file"
sudo grep -Fxq "CREG_NODE_ROLE=$role" "$env_file"
sudo grep -Fxq "CREG_OSV_SNAPSHOT_EPOCH=$old_epoch" "$env_file"
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$old_profile" "$env_file"
sudo grep -Fxq 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$env_file"
! sudo grep -q '^CREG_PHASE_C_COMMIT_CAPTURE_' "$env_file"
! sudo docker container inspect "$preserved" >/dev/null 2>&1

sudo python3 - "$container" "$data" "$old_scanner" "$api_port" "$role" "$node_id" "$old_profile" "$chain_id" "$genesis" "$tip_hash" >/tmp/creg-r13-$slot-preflight.json <<'PY'
import json,re,subprocess,sys,urllib.request
container,data,scanner,port,role,node_id,profile,chain,genesis,tip=sys.argv[1:]
info=json.loads(subprocess.check_output(['docker','inspect',container]))[0]
mounts={(m['Source'],m['Destination'],m['RW']) for m in info['Mounts']}
assert (data,'/data',True) in mounts and (scanner,'/scanner',False) in mounts
def get(path):
 with urllib.request.urlopen(f'http://127.0.0.1:{port}{path}',timeout=8) as r:return r.read().decode()
stats=json.loads(get('/v1/public/chain/stats'));health=json.loads(get('/v1/public/health'))
pbft=json.loads(get('/v1/consensus/pbft'));metrics=get('/metrics')
assert stats.get('chain_id')==chain and stats.get('genesis_hash','').removeprefix('0x')==genesis
assert stats.get('tip_height')==stats.get('finalized_height')==16 and stats.get('tip_hash')==tip
assert stats.get('spec_signature_verified') is True and stats.get('validator_count')==stats.get('active_validators')==4
assert stats.get('peer_count',0)>0 and health.get('node_id')==node_id and health.get('is_validator') is (role=='validator')
assert f'profile_digest="{profile}"' in metrics and 'creg_scanner_profile_ready 1' in metrics
assert 'creg_scanner_osv_lookups 6' in metrics and 'creg_scanner_osv_ready 1' in metrics
def value(name):
 m=re.search(r'^'+re.escape(name)+r'(?:\{[^}]*\})?\s+([-+]?[0-9.]+)$',metrics,re.M)
 return float(m.group(1)) if m else None
assert all(value(n)==0 for n in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending'))
assert not [r for r in pbft.get('rounds',[]) if r.get('phase')!='finalised' and (r.get('phase')!='failed' or r.get('prepare_count',0)>=r.get('quorum',3))]
dead=value('creg_durable_finalized_work_dead_letter_depth');assert dead is not None
print(json.dumps({'stage':'preflight','node_id':node_id,'image_id':info['Image'],'height':16,'block_hash':tip,'dead_letters':dead,'ready':True}))
PY
dead_before=$(python3 -c 'import json; print(json.load(open("/tmp/creg-r13-'"$slot"'-preflight.json"))["dead_letters"])')

for spec in \
  'osv_snapshot.json 642c35a82a9296c36bfc5b79c152b3a78651d7c50d646206bc004620caa0925a' \
  'manifest.json 510c237d49a709a57606427cdbbda6caa989864b8a8e0313b797eb29cd25eeab' \
  'known_malicious_hashes.json c97381b9df48722a1f591c8f81c87e4d9695c0b89877e6b609e883abf7e4a723' \
  'rules/supply_chain.yar 217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e'; do
  set -- $spec
  printf '%s  %s\n' "$2" "$stage/$1" | sha256sum -c - >/dev/null
done
sudo install -d -m 0755 "$scanner/rules" "$base/evidence"
sudo install -m 0644 "$stage/known_malicious_hashes.json" "$scanner/known_malicious_hashes.json"
sudo install -m 0644 "$stage/manifest.json" "$scanner/manifest.json"
sudo install -m 0644 "$stage/osv_snapshot.json" "$scanner/osv_snapshot.json"
sudo install -m 0644 "$stage/rules/supply_chain.yar" "$scanner/rules/supply_chain.yar"
sudo cp -p "$env_file" "$capture_env"
sudo python3 - "$env_file" "$old_epoch" "$old_profile" "$epoch" "$profile" <<'PY'
import os,pathlib,sys,tempfile
path,old_epoch,old_profile,epoch,profile=sys.argv[1:]
p=pathlib.Path(path);text=p.read_bytes().decode()
for key,old,new in (('CREG_OSV_SNAPSHOT_EPOCH',old_epoch,epoch),('CREG_SCANNER_PROFILE_DIGEST',old_profile,profile)):
 before=key+'='+old;after=key+'='+new
 if text.splitlines().count(before)!=1:raise SystemExit('Unexpected prior scanner env value for '+key)
 text=text.replace(before,after,1)
fd,tmp=tempfile.mkstemp(prefix='.r13-node-env-',dir=str(p.parent))
try:
 os.fchmod(fd,0o600)
 with os.fdopen(fd,'wb') as out:out.write(text.encode());out.flush();os.fsync(out.fileno())
 os.replace(tmp,p)
finally:
 if os.path.exists(tmp):os.unlink(tmp)
PY

identity_before=$(sudo sha256sum "$data/p2p/identity.key" | cut -d' ' -f1)
env_before=$(sudo sha256sum "$env_file" | cut -d' ' -f1)
sudo docker stop --time 30 "$container" >/dev/null
sudo docker rename "$container" "$preserved"
args=(run -d --name "$container" --restart unless-stopped --env-file "$env_file"
  -p "$api_port:8080" -p "$p2p_port:9000"
  --mount "type=bind,source=$data,target=/data"
  --mount "type=bind,source=$scanner,target=/scanner,readonly")
if [[ "$role" == validator ]]; then
  docker_group=$(stat -c '%g' /var/run/docker.sock)
  args+=(--privileged --group-add "$docker_group" --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock)
fi
sudo docker "${args[@]}" "$expected_image" >/dev/null

deadline=$((SECONDS+360))
while (( SECONDS < deadline )); do
  if sudo python3 - "$container" "$preserved" "$data" "$scanner" "$env_file" "$api_port" "$role" "$node_id" "$expected_image" "$identity_before" "$env_before" "$epoch" "$profile" "$chain_id" "$genesis" "$tip_hash" "$dead_before" >/tmp/creg-r13-$slot-check.json 2>/tmp/creg-r13-$slot-check-error.log <<'PY'
import hashlib,json,re,subprocess,sys,urllib.request
container,preserved,data,scanner,env_path,port,role,node_id,image,identity,env_hash,epoch,profile,chain,genesis,tip,dead_before=sys.argv[1:]
def sh(*args):return subprocess.check_output(args,text=True).strip()
def sha(path):
 h=hashlib.sha256()
 with open(path,'rb') as f:
  for b in iter(lambda:f.read(1048576),b''):h.update(b)
 return h.hexdigest()
def get(path):
 with urllib.request.urlopen(f'http://127.0.0.1:{port}{path}',timeout=8) as r:return r.read().decode()
info=json.loads(sh('docker','inspect',container))[0];old=json.loads(sh('docker','inspect',preserved))[0]
stats=json.loads(get('/v1/public/chain/stats'));health=json.loads(get('/v1/public/health'));pbft=json.loads(get('/v1/consensus/pbft'));metrics=get('/metrics')
mounts={(m['Source'],m['Destination'],m['RW']) for m in info['Mounts']}
binary=sh('docker','exec',container,'sha256sum','/app/creg-node').split()[0]
def value(name):
 m=re.search(r'^'+re.escape(name)+r'(?:\{[^}]*\})?\s+([-+]?[0-9.]+)$',metrics,re.M)
 return float(m.group(1)) if m else None
unresolved=[r for r in pbft.get('rounds',[]) if r.get('phase')!='finalised' and (r.get('phase')!='failed' or r.get('prepare_count',0)>=r.get('quorum',3))]
dead=value('creg_durable_finalized_work_dead_letter_depth')
checks={'image_preserved':info['Image']==image,'new_running_healthy':info['State']['Running'] and info['State']['Health']['Status']=='healthy','old_container_stopped':old['State']['Status']=='exited','data_and_scanner_mounts':(data,'/data',True) in mounts and (scanner,'/scanner',False) in mounts,'same_identity_and_env':sha(data+'/p2p/identity.key')==identity and sha(env_path)==env_hash,'signed_r7_h16':stats.get('chain_id')==chain and stats.get('genesis_hash','').removeprefix('0x')==genesis and stats.get('tip_height')==stats.get('finalized_height')==16 and stats.get('tip_hash')==tip and stats.get('spec_signature_verified') is True,'four_validator_roster':stats.get('validator_count')==stats.get('active_validators')==4,'role_and_peers':health.get('node_id')==node_id and health.get('is_validator') is (role=='validator') and stats.get('peer_count',0)>0,'scanner_profile':f'profile_digest="{profile}"' in metrics and value('creg_scanner_profile_ready')==1 and value('creg_scanner_osv_ready')==1 and value('creg_scanner_osv_lookups')==7 and value('creg_scanner_threat_intel_entries')==100,'queues_clear':all(value(n)==0 for n in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')),'dead_letters_preserved':dead==float(dead_before),'pbft_clear':not unresolved}
if not all(checks.values()):raise SystemExit(json.dumps({k:v for k,v in checks.items() if not v}))
print(json.dumps({'slot':node_id,'image_id':info['Image'],'node_binary_sha256':binary,'height':16,'block_hash':tip,'peer_count':stats['peer_count'],'scanner_profile_digest':profile,'dead_letter_depth':dead,'checks':checks,'passed':True,'preserved_container':preserved}))
PY
  then
    sudo install -m 0644 /tmp/creg-r13-$slot-check.json "$evidence"
    cat /tmp/creg-r13-$slot-check.json
    exit 0
  fi
  sleep 3
done
sudo install -m 0644 /tmp/creg-r13-$slot-check.json "$base/evidence/r13-$slot-failed-postcheck-$run_id.json"
echo 'R13 postflight failed; prior container and signing state are preserved. Stop fleet rollout and inspect the evidence.' >&2
cat /tmp/creg-r13-$slot-check-error.log >&2
exit 1
