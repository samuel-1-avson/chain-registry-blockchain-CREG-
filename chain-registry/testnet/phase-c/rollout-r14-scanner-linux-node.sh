#!/usr/bin/env bash
# Replace one real R7 node with an immutable R14 OSV profile. The node image,
# chain store, P2P identity, PBFT signer journal and network attachments persist.
set -euo pipefail

slot=${1:?slot}; role=${2:?role}; api_port=${3:?API port}; p2p_port=${4:?P2P port}
node_id=${5:?node ID}; expected_image=${6:?current image ID}
profile=${7:?new scanner profile}; epoch=${8:?new OSV epoch}
snapshot_sha=${9:?OSV snapshot SHA256}; manifest_sha=${10:?manifest SHA256}
threat_sha=${11:?threat database SHA256}; yara_sha=${12:?YARA bundle SHA256}
stage=${13:?unique scanner staging directory}

run_id=$(date -u +%Y%m%dT%H%M%SZ)
base=/var/lib/creg/phase-c-r7; data=$base/data; env_file=$base/node.env
scanner=$base/scanner-r14-ansi-regex-live
container=creg-phase-c-r7-$slot; preserved=$container-r13-before-r14-$run_id
failed=$container-r14-failed-$run_id
env_backup=$base/node.env.pre-r14-$run_id
evidence=$base/evidence/r14-$slot-rollout-$run_id.json
chain_id=creg-phase-c-testnet-2026-09-26-r7
network_genesis=0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4
signed_spec_genesis=1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234
shared_hash=ec64471b8f9c58eb2d9b958db765a98871cb444e032143d313d65b3ab80cd76c
old_profile=8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e
old_epoch=osv-2026-09-28-phase-c-has-flag-r13-live
canonical=npm:ansi-regex@6.1.0

case "$slot:$role:$api_port:$p2p_port:$node_id" in
  v2:validator:18080:9001:validator-2|v3:validator:8080:9000:validator-3|v4:validator:8080:9000:validator-4|o1:observer:18080:9001:observer-1|o2:observer:8080:9000:observer-2|o3:observer:8080:9000:observer-3|o4:observer:8080:9000:observer-4) ;;
  *) echo 'Unexpected R7 node identity or port map' >&2; exit 2 ;;
esac
exec 9>"/tmp/creg-r14-$slot-rollout.lock"
flock -n 9 || { echo 'R14 rollout already active for this node' >&2; exit 1; }
[[ $(sudo docker inspect "$container" --format '{{.Image}} {{.State.Status}}') == "$expected_image running" ]]
[[ -d "$data/chain.rocksdb" && -f "$data/p2p/identity.key" && -f "$env_file" ]]
if [[ "$role" == validator ]]; then [[ -d "$data/pbft-safety.rocksdb" ]]; fi
[[ ! -e "$scanner" && ! -e "$env_backup" && ! -e "$evidence" && ! -e "$preserved" && ! -e "$failed" ]]
sudo grep -Fxq "CREG_NODE_ID=$node_id" "$env_file"
sudo grep -Fxq "CREG_NODE_ROLE=$role" "$env_file"
sudo grep -Fxq "CREG_OSV_SNAPSHOT_EPOCH=$old_epoch" "$env_file"
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$old_profile" "$env_file"
sudo grep -Fxq 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$env_file"
! sudo grep -q '^CREG_PHASE_C_COMMIT_CAPTURE_' "$env_file"

for spec in \
  "osv_snapshot.json $snapshot_sha" \
  "manifest.json $manifest_sha" \
  "known_malicious_hashes.json $threat_sha" \
  "rules/supply_chain.yar $yara_sha"; do
  set -- $spec
  printf '%s  %s\n' "$2" "$stage/$1" | sha256sum -c - >/dev/null
done

sudo python3 - "$stage" "$profile" "$epoch" "$canonical" "$snapshot_sha" "$manifest_sha" "$threat_sha" "$yara_sha" <<'PY'
import hashlib,json,pathlib,sys
root=pathlib.Path(sys.argv[1]);profile,epoch,canonical,snapshot_sha,manifest_sha,threat_sha,yara_sha=sys.argv[2:]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
manifest=json.loads((root/'manifest.json').read_text());snapshot=json.loads((root/'osv_snapshot.json').read_text())
assert manifest.get('scanner_profile_digest')==profile and manifest.get('osv_epoch')==epoch
assert snapshot.get('epoch')==epoch and canonical in snapshot.get('entries',{})
assert sha(root/'osv_snapshot.json')==snapshot_sha and sha(root/'manifest.json')==manifest_sha
assert sha(root/'known_malicious_hashes.json')==threat_sha and sha(root/'rules/supply_chain.yar')==yara_sha
files=sorted((root/'rules').glob('*.yar'),key=lambda p:p.name);h=hashlib.sha256();h.update(b'creg-yara-rule-bundle-v1\0')
for path in files:
 for item in (path.name.encode(),path.read_bytes()):h.update(len(item).to_bytes(8,'big'));h.update(item)
yara_bundle=h.hexdigest();threat=sha(root/'known_malicious_hashes.json');osv=sha(root/'osv_snapshot.json')
identity=f'creg-scanner-profile-v1|version=creg-deterministic-scanner-v1.1.0|yara={yara_bundle}|threat={threat}|osv_enabled=true|osv_epoch={epoch}|osv={osv}|osv_block_critical=true|osv_live_fallback=false'
assert manifest.get('yara_bundle_sha256')==yara_bundle
assert hashlib.sha256(identity.encode()).hexdigest()==profile
assert len(snapshot['entries'])==8
PY

sudo cp -p "$env_file" "$env_backup"
identity_before=$(sudo sha256sum "$data/p2p/identity.key" | cut -d' ' -f1)
# Capture exact live state before changing the scanner environment or container.
sudo python3 - "$container" "$data" "$env_file" "$api_port" "$role" "$node_id" "$expected_image" "$old_profile" "$old_epoch" "$chain_id" "$network_genesis" "$shared_hash" "$canonical" >"/tmp/creg-r14-$slot-preflight.json" <<'PY'
import hashlib,json,re,subprocess,sys,urllib.error,urllib.parse,urllib.request
container,data,env_path,port,role,node_id,image,profile,old_epoch,chain,genesis,tip,canonical=sys.argv[1:]
def sh(*args):return subprocess.check_output(args,text=True).strip()
def get(path):
 with urllib.request.urlopen(f'http://127.0.0.1:{port}{path}',timeout=12) as r:return r.status,r.read(65536).decode('utf8','replace')
history=None
def metric(text,name):
 m=re.search(r'^'+re.escape(name)+r'(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+))$',text,re.M);return float(m.group(1)) if m else None
info=json.loads(sh('docker','inspect',container))[0];stats=json.loads(get('/v1/public/chain/stats')[1]);health=json.loads(get('/v1/public/health')[1]);pbft=json.loads(get('/v1/consensus/pbft')[1]);metrics=get('/metrics')[1]
try:package_status=get('/v1/public/packages/'+urllib.parse.quote(canonical,safe=''))[0]
except urllib.error.HTTPError as e:package_status=e.code
if node_id=='observer-1':history=json.loads(get('/v1/public/metrics/history?range=30m')[1])
def digest(path):
 h=hashlib.sha256()
 with open(path,'rb') as f:
  for b in iter(lambda:f.read(1048576),b''):h.update(b)
 return h.hexdigest()
unresolved=[r for r in pbft.get('rounds',[]) if r.get('phase')!='finalised' and (r.get('phase')!='failed' or r.get('prepare_count',0)>=r.get('quorum',3))]
mounts={(m['Source'],m['Destination'],m['RW']) for m in info.get('Mounts',[])}
container_env=dict(item.split('=',1) for item in info.get('Config',{}).get('Env',[]) if '=' in item)
checks={'container_running':info.get('State',{}).get('Running') is True,'expected_image':info.get('Image')==image,
 'persistent_mounts':(data,'/data',True) in mounts and any(dest=='/scanner' and source.startswith('/var/lib/creg/phase-c-r7/scanner-') and rw is False for source,dest,rw in mounts),
 'container_environment':container_env.get('CREG_NODE_ID')==node_id and container_env.get('CREG_NODE_ROLE')==role and container_env.get('CREG_SCANNER_PROFILE_DIGEST')==profile and container_env.get('CREG_OSV_SNAPSHOT_EPOCH')==old_epoch and container_env.get('CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM','').lower()=='false' and not any(k.startswith('CREG_PHASE_C_COMMIT_CAPTURE_') for k in container_env),
 'health_and_identity':health.get('status')=='ok' and health.get('node_id')==node_id and health.get('is_validator') is (role=='validator'),
 'signed_r7_tip_unchanged':stats.get('chain_id')==chain and stats.get('genesis_hash','').removeprefix('0x')==genesis and stats.get('spec_signature_verified') is True and stats.get('tip_height')==stats.get('finalized_height')==18 and stats.get('tip_hash','').removeprefix('0x')==tip,
 'four_validator_roster_and_peer':stats.get('validator_count')==stats.get('active_validators')==4 and stats.get('peer_count',0)>0,
 'package_still_absent':package_status==404,'old_scanner_profile':f'profile_digest="{profile}"' in metrics,
 'queues_clear':all(metric(metrics,n)==0 for n in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')),'pbft_clear':not unresolved}
if not all(checks.values()):raise SystemExit(json.dumps({k:v for k,v in checks.items() if not v}))
report={'node_id':node_id,'image_id':info['Image'],'height':18,'block_hash':tip,'scanner_profile_digest':profile,'dead_letter_depth':metric(metrics,'creg_durable_finalized_work_dead_letter_depth'),'identity_sha256':digest(data+'/p2p/identity.key'),'env_sha256':digest(env_path),'networks':info.get('NetworkSettings',{}).get('Networks',{}),'restart_policy':info.get('HostConfig',{}).get('RestartPolicy',{}).get('Name'),'network_mode':info.get('HostConfig',{}).get('NetworkMode'),'checks':checks}
print(json.dumps(report))
PY
sudo chmod 0600 "/tmp/creg-r14-$slot-preflight.json"
dead_before=$(sudo python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["dead_letter_depth"])' "/tmp/creg-r14-$slot-preflight.json")
network_mode=$(sudo python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["network_mode"] or "bridge")' "/tmp/creg-r14-$slot-preflight.json")
restart_policy=$(sudo python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["restart_policy"] or "no")' "/tmp/creg-r14-$slot-preflight.json")
network_names=$(sudo python3 -c 'import json,sys;print("\n".join(json.load(open(sys.argv[1]))["networks"].keys()))' "/tmp/creg-r14-$slot-preflight.json")

rollback_r14() {
  rc=$?
  trap - ERR
  set +e
  if sudo docker container inspect "$preserved" >/dev/null 2>&1; then
    if sudo docker container inspect "$container" >/dev/null 2>&1; then
      sudo docker stop --time 20 "$container" >/dev/null 2>&1
      sudo docker rename "$container" "$failed" >/dev/null 2>&1
    fi
    sudo cp -p "$env_backup" "$env_file"
    sudo docker rename "$preserved" "$container"
    sudo docker start "$container" >/dev/null
    echo 'R14 rollback restored the preserved node with its original scanner and persistent state.' >&2
  elif [[ -f "$env_backup" ]]; then
    sudo cp -p "$env_backup" "$env_file"
  fi
  exit "$rc"
}
trap rollback_r14 ERR

sudo install -d -m 0755 "$scanner/rules" "$base/evidence"
sudo install -m 0644 "$stage/known_malicious_hashes.json" "$scanner/known_malicious_hashes.json"
sudo install -m 0644 "$stage/manifest.json" "$scanner/manifest.json"
sudo install -m 0644 "$stage/osv_snapshot.json" "$scanner/osv_snapshot.json"
sudo install -m 0644 "$stage/rules/supply_chain.yar" "$scanner/rules/supply_chain.yar"
sudo python3 - "$env_file" "$old_epoch" "$old_profile" "$epoch" "$profile" <<'PY'
import os,pathlib,sys,tempfile
path,old_epoch,old_profile,epoch,profile=sys.argv[1:]
p=pathlib.Path(path);text=p.read_bytes().decode()
for key,old,new in (('CREG_OSV_SNAPSHOT_EPOCH',old_epoch,epoch),('CREG_SCANNER_PROFILE_DIGEST',old_profile,profile)):
 before=key+'='+old;after=key+'='+new
 if text.splitlines().count(before)!=1:raise SystemExit('Unexpected prior scanner env value for '+key)
 text=text.replace(before,after,1)
fd,tmp=tempfile.mkstemp(prefix='.r14-node-env-',dir=str(p.parent))
try:
 os.fchmod(fd,0o600)
 with os.fdopen(fd,'wb') as out:out.write(text.encode());out.flush();os.fsync(out.fileno())
 os.replace(tmp,p)
finally:
 if os.path.exists(tmp):os.unlink(tmp)
PY

sudo docker stop --time 30 "$container" >/dev/null
sudo docker rename "$container" "$preserved"
args=(run -d --name "$container" --restart "$restart_policy" --env-file "$env_file"
  --mount "type=bind,source=$data,target=/data"
  --mount "type=bind,source=$scanner,target=/scanner,readonly")
if [[ "$network_mode" == host ]]; then
  args+=(--network host)
else
  args+=(-p "$api_port:8080" -p "$p2p_port:9000")
  if [[ "$network_mode" != bridge && "$network_mode" != default && "$network_mode" != none ]]; then args+=(--network "$network_mode"); fi
fi
if [[ "$role" == validator ]]; then
  docker_group=$(stat -c '%g' /var/run/docker.sock)
  args+=(--privileged --group-add "$docker_group" --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock)
fi
sudo docker "${args[@]}" "$expected_image" >/dev/null

while IFS= read -r network; do
  [[ -z "$network" || "$network" == bridge || "$network" == "$network_mode" ]] && continue
  aliases=$(sudo python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));print("\n".join(d["networks"].get(sys.argv[2],{}).get("Aliases",[]) or []))' "/tmp/creg-r14-$slot-preflight.json" "$network")
  alias_args=()
  while IFS= read -r alias; do [[ -n "$alias" ]] && alias_args+=(--alias "$alias"); done <<< "$aliases"
  sudo docker network connect "${alias_args[@]}" "$network" "$container"
done <<< "$network_names"

deadline=$((SECONDS+360))
while (( SECONDS < deadline )); do
  if sudo python3 - "$container" "$preserved" "$data" "$scanner" "$env_file" "$api_port" "$role" "$node_id" "$expected_image" "$identity_before" "$env_backup" "$epoch" "$profile" "$chain_id" "$network_genesis" "$shared_hash" "$canonical" "$dead_before" "$network_mode" "/tmp/creg-r14-$slot-preflight.json" "$signed_spec_genesis" >"/tmp/creg-r14-$slot-check.json" 2>"/tmp/creg-r14-$slot-check-error.log" <<'PY'
import datetime,hashlib,json,os,re,subprocess,sys,urllib.error,urllib.parse,urllib.request
container,preserved,data,scanner,env_path,port,role,node_id,image,identity,env_backup,epoch,profile,chain,genesis,tip,canonical,dead_before,network_mode,preflight_path,signed_genesis=sys.argv[1:]
def sh(*args):return subprocess.check_output(args,text=True).strip()
def sha(path):
 h=hashlib.sha256()
 with open(path,'rb') as f:
  for b in iter(lambda:f.read(1048576),b''):h.update(b)
 return h.hexdigest()
def get(path):
 with urllib.request.urlopen(f'http://127.0.0.1:{port}{path}',timeout=8) as r:return r.status,r.read(65536).decode('utf8','replace')
history=None
def metric(text,name):
 m=re.search(r'^'+re.escape(name)+r'(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+))$',text,re.M);return float(m.group(1)) if m else None
info=json.loads(sh('docker','inspect',container))[0];old=json.loads(sh('docker','inspect',preserved))[0]
stats=json.loads(get('/v1/public/chain/stats')[1]);health=json.loads(get('/v1/public/health')[1]);pbft=json.loads(get('/v1/consensus/pbft')[1]);metrics=get('/metrics')[1]
try:package_status=get('/v1/public/packages/'+urllib.parse.quote(canonical,safe=''))[0]
except urllib.error.HTTPError as e:package_status=e.code
if node_id=='observer-1':history=json.loads(get('/v1/public/metrics/history?range=5m')[1])
mounts={(m['Source'],m['Destination'],m['RW']) for m in info['Mounts']};pre=json.load(open(preflight_path));network_names=sorted(info.get('NetworkSettings',{}).get('Networks',{}))
def unresolved_rounds():return [r for r in pbft.get('rounds',[]) if r.get('phase')!='finalised' and (r.get('phase')!='failed' or r.get('prepare_count',0)>=r.get('quorum',3))]
checks={'image_unchanged':info.get('Image')==image and info.get('State',{}).get('Running') is True and info.get('State',{}).get('Health',{}).get('Status')=='healthy','old_container_preserved_stopped':old.get('State',{}).get('Status')=='exited',
 'data_and_scanner_mounts':(data,'/data',True) in mounts and (scanner,'/scanner',False) in mounts,
 'identity_and_signer_journal_paths_preserved':sha(data+'/p2p/identity.key')==identity and (role!='validator' or os.path.isdir(data+'/pbft-safety.rocksdb')),
 'signed_r7_tip_unchanged':stats.get('chain_id')==chain and stats.get('genesis_hash','').removeprefix('0x')==genesis and stats.get('spec_signature_verified') is True and stats.get('tip_height')==stats.get('finalized_height')==18 and stats.get('tip_hash','').removeprefix('0x')==tip,
 'four_validators_and_live_peer':stats.get('validator_count')==stats.get('active_validators')==4 and stats.get('peer_count',0)>0,
 'health_and_role':health.get('status')=='ok' and health.get('node_id')==node_id and health.get('is_validator') is (role=='validator'),
 'package_still_absent':package_status==404,'r14_profile_ready':f'profile_digest="{profile}"' in metrics and metric(metrics,'creg_scanner_profile_ready')==1 and metric(metrics,'creg_scanner_osv_ready')==1 and metric(metrics,'creg_scanner_osv_lookups')==8,
 'threat_feed_ready':metric(metrics,'creg_scanner_threat_intel_ready')==1 and metric(metrics,'creg_scanner_threat_intel_entries')==100,
 'queues_clear':all(metric(metrics,n)==0 for n in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')),
 'dead_letters_preserved':metric(metrics,'creg_durable_finalized_work_dead_letter_depth')==float(dead_before),'pbft_clear':not unresolved_rounds(),
 'network_attachments_preserved':network_names==sorted(pre['networks'].keys()),
 'o1_durable_history_ready':node_id!='observer-1' or (history.get('status')=='ready' and history.get('collector')=='prometheus' and history.get('queried_series')==7 and history.get('sample_count',0)>=3 and history.get('chain_id')==chain and history.get('network_genesis_hash','').removeprefix('0x')==signed_genesis),
 'restart_policy_preserved':info.get('HostConfig',{}).get('RestartPolicy',{}).get('Name')==pre.get('restart_policy','no')}
if not all(checks.values()):raise SystemExit(json.dumps({k:v for k,v in checks.items() if not v}))
report={'stage':'postflight','captured_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'node_id':node_id,'image_id':info['Image'],'node_binary_sha256':sh('docker','exec',container,'sha256sum','/app/creg-node').split()[0],'height':18,'block_hash':tip,'scanner_profile_digest':profile,'dead_letter_depth':metric(metrics,'creg_durable_finalized_work_dead_letter_depth'),'network_names':network_names,'history_status':history.get('status') if history else None,'history_samples':history.get('sample_count') if history else None,'checks':checks,'passed':True,'preserved_container':preserved}
print(json.dumps(report))
PY
  then
    sudo install -m 0644 "/tmp/creg-r14-$slot-check.json" "$evidence"
    cat "/tmp/creg-r14-$slot-check.json"
    trap - ERR
    exit 0
  fi
  sleep 3
done

sudo install -m 0644 "/tmp/creg-r14-$slot-check.json" "$base/evidence/r14-$slot-failed-postcheck-$run_id.json"
echo 'R14 postflight failed. Preserving both containers and restoring the original scanner configuration.' >&2
trap - ERR
sudo docker stop --time 30 "$container" >/dev/null 2>&1 || true
sudo docker rename "$container" "$failed" >/dev/null 2>&1 || true
sudo cp -p "$env_backup" "$env_file"
sudo docker rename "$preserved" "$container"
sudo docker start "$container" >/dev/null
echo 'Original container restarted with its original data, identity, signer journal, scanner profile and networks.' >&2
cat "/tmp/creg-r14-$slot-check.json" >&2
cat "/tmp/creg-r14-$slot-check-error.log" >&2
exit 1
