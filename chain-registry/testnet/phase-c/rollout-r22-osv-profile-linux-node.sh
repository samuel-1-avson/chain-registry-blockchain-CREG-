#!/usr/bin/env bash
set -euo pipefail
slot=$1; role=$2; api_port=$3; p2p_port=$4; node_id=$5; dead=$6; old_epoch=$7; old_profile=$8
shift 8
new_epoch=$1; shift; new_profile=$1; shift; osv_sha=$1; shift; manifest_sha=$1; shift; image_id=$1; shift; registry_ref=$1
case "$slot:$role" in v[2-4]:validator|o[1-4]:observer) ;; *) exit 2 ;; esac
case "$api_port:$p2p_port" in 18080:9001|8080:9000) ;; *) exit 2 ;; esac
[[ "$node_id" == "$role-$(printf '%s' "$slot" | cut -c2-)" ]]
[[ "$dead" =~ ^[0-9]+$ ]]
case "$image_id" in sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3|sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9) ;; *) exit 2 ;; esac
[[ "$registry_ref" == 'us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3' ]]
[[ "$osv_sha" == '452b326cc8d7bea568f21e2fd0d42d9b45503f481a8efe26a174ecb09cabe930' ]]
[[ "$manifest_sha" == 'bbaca082dbcece3fd4ef808b17057a27e7862135921bf694a1d2ff458518d932' ]]
[[ "$old_epoch" == 'osv-2026-09-26-phase-c-r10-event-check' ]]
[[ "$old_profile" == 'd09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4' ]]
[[ "$new_epoch" == 'osv-2026-09-27-phase-c-kleur-r11' ]]
[[ "$new_profile" == 'f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c' ]]
container="creg-phase-c-r7-$slot"; base=/var/lib/creg/phase-c-r7; data="$base/data"
env_file="$base/node.env"; backup="$base/node.env.osv-r10-preserved"; old_scanner="$base/scanner-r18-osv-r10"
scanner="$base/scanner-r11-osv-kleur"; stage="/tmp/creg-scanner-r11-$slot"
preserved="$container-r22-before-osv-r11"; evidence="$base/evidence/scanner-r11-$slot-rollout.json"
image=creg-node:phase-c-secure-r22-commit-capture; chain=creg-phase-c-testnet-2026-09-26-r7
genesis=0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4
tip=a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d
api="http://127.0.0.1:$api_port"; started=false
exec 9>"/tmp/creg-scanner-r11-$slot.lock"; flock -n 9
for path in "$env_file" "$data/p2p/identity.key" "$data/chain.rocksdb" "$old_scanner" "$stage"; do sudo test -e "$path"; done
[[ ! -e "$scanner" && ! -e "$backup" && ! -e "$evidence" ]]
! sudo docker container inspect "$preserved" >/dev/null 2>&1
sudo grep -Fxq "CREG_NODE_ID=$node_id" "$env_file"
sudo grep -Fxq "CREG_NODE_ROLE=$role" "$env_file"
sudo grep -Fxq 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$env_file"
sudo grep -Fxq "CREG_OSV_SNAPSHOT_EPOCH=$old_epoch" "$env_file"
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$old_profile" "$env_file"
[[ $(sudo docker inspect "$container" --format '{{.Image}}') == "$image_id" ]]
digests=$(sudo docker image inspect "$image_id" --format '{{join .RepoDigests "\n"}}')
short_registry_ref="creg-node@${registry_ref#*@}"
grep -Fxq "$registry_ref" <<<"$digests" || grep -Fxq "$short_registry_ref" <<<"$digests"
printf '%s  %s\n' 'eda8108603d2d674fcdc385635f2e7ac3091a044322214d1c1568b79de8986b9' "$old_scanner/osv_snapshot.json" | sudo sha256sum -c - >/dev/null
printf '%s  %s\n' '452b326cc8d7bea568f21e2fd0d42d9b45503f481a8efe26a174ecb09cabe930' "$stage/osv_snapshot.json" | sha256sum -c - >/dev/null
printf '%s  %s\n' 'bbaca082dbcece3fd4ef808b17057a27e7862135921bf694a1d2ff458518d932' "$stage/manifest.json" | sha256sum -c - >/dev/null
printf '%s  %s\n' '5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef' "$stage/known_malicious_hashes.json" | sha256sum -c - >/dev/null
printf '%s  %s\n' '217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e' "$stage/rules/supply_chain.yar" | sha256sum -c - >/dev/null
python3 - "$stage/manifest.json" "$stage/osv_snapshot.json" "$new_epoch" "$new_profile" <<'PY'
import json,sys
m=json.load(open(sys.argv[1]));s=json.load(open(sys.argv[2]))
assert m.get('osv_epoch')==sys.argv[3] and s.get('epoch')==sys.argv[3]
assert m.get('scanner_profile_digest')==sys.argv[4]
assert s.get('entries',{}).get('npm:kleur@4.1.5')==[] and len(m.get('osv_coverage',[]))==5
PY
python3 - "$api" "$role" "$old_epoch" "$old_profile" "$dead" "$chain" "$genesis" "$tip" <<'PY'
import json,sys,urllib.request
api,role,epoch,profile,dead,chain,genesis,tip=sys.argv[1:]
def get(p):
 with urllib.request.urlopen(api+p,timeout=10) as r:return r.read().decode()
s=json.loads(get('/v1/public/chain/stats'));h=json.loads(get('/v1/public/health'));t=get('/metrics')
assert s.get('chain_id')==chain and s.get('tip_height')==12 and s.get('tip_hash')==tip
assert h.get('is_validator') is (role=='validator')
assert 'profile_digest="'+profile+'"' in t
PY
mounts=$(sudo docker inspect "$container" --format '{{range .Mounts}}{{println .Source "|" .Destination "|" .RW}}{{end}}')
grep -Fq "$data | /data | true" <<<"$mounts"
grep -Fq "$old_scanner | /scanner | false" <<<"$mounts"
python3 - "$api" "$role" "$old_epoch" "$old_profile" "$dead" "$chain" "$genesis" "$tip" <<'PY'
import json,re,sys,urllib.request
api,role,epoch,profile,dead,chain,genesis,tip=sys.argv[1:]
def get(p):
 with urllib.request.urlopen(api+p,timeout=10) as r:return r.read().decode()
s=json.loads(get('/v1/public/chain/stats'));h=json.loads(get('/v1/public/health'));t=get('/metrics')
rounds=json.loads(get('/v1/consensus/pbft')).get('rounds',[])
def metric(n):
 m=re.search(r'^'+re.escape(n)+r'(?:\{[^}]*\})?\s+([0-9.]+)$',t,re.M);return float(m.group(1)) if m else None
assert s.get('chain_id')==chain and s.get('spec_signature_verified') is True
assert s.get('genesis_hash','').removeprefix('0x')==genesis
assert s.get('tip_height')==12 and s.get('finalized_height')==12 and s.get('tip_hash')==tip and s.get('package_count')==4
assert s.get('peer_count',0)>0 and s.get('validator_count')==4 and s.get('active_validators')==4
assert h.get('is_validator') is (role=='validator')
assert f'profile_digest="{profile}"' in t
assert metric('creg_scanner_profile_ready')==1
assert metric('creg_durable_finalized_work_dead_letter_depth')==float(dead)
assert all(metric(n)==0 for n in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending'))
assert all(r.get('phase') in ('finalised','failed') for r in rounds)
PY
identity=$(sudo sha256sum "$data/p2p/identity.key" | cut -d' ' -f1)
sudo install -d -m 0755 "$scanner/rules" "$base/evidence"
sudo install -m 0644 "$stage/known_malicious_hashes.json" "$scanner/known_malicious_hashes.json"
sudo install -m 0644 "$stage/manifest.json" "$scanner/manifest.json"
sudo install -m 0644 "$stage/osv_snapshot.json" "$scanner/osv_snapshot.json"
sudo install -m 0644 "$stage/rules/supply_chain.yar" "$scanner/rules/supply_chain.yar"
sudo cp -p "$env_file" "$backup"
sudo python3 - "$env_file" "$old_epoch" "$old_profile" "$new_epoch" "$new_profile" <<'PY'
import pathlib,sys
p=pathlib.Path(sys.argv[1]);oe,op,ne,np=sys.argv[2:];text=p.read_text()
for k,o,n in [('CREG_OSV_SNAPSHOT_EPOCH',oe,ne),('CREG_SCANNER_PROFILE_DIGEST',op,np)]:
 old=k+'='+o;assert text.count(old)==1;text=text.replace(old,k+'='+n,1)
p.write_text(text)
PY
rollback() {
 status=$?
 if (( status != 0 )) && [[ "$started" == false ]]; then
   sudo cp -p "$backup" "$env_file" 2>/dev/null || true
   if sudo docker container inspect "$preserved" >/dev/null 2>&1 && ! sudo docker container inspect "$container" >/dev/null 2>&1; then
     sudo docker rename "$preserved" "$container" 2>/dev/null || true
     sudo docker start "$container" >/dev/null 2>&1 || true
   fi
 fi
 exit "$status"
}
trap rollback EXIT
sudo docker stop --time 20 "$container" >/dev/null
sudo docker rename "$container" "$preserved"
if [[ "$role" == validator ]]; then
 group=$(stat -c '%g' /var/run/docker.sock)
 sudo docker run -d --name "$container" --restart unless-stopped --env-file "$env_file" \
  -p "$api_port:8080" -p "$p2p_port:9000" \
  --mount "type=bind,source=$data,target=/data" \
  --mount "type=bind,source=$scanner,target=/scanner,readonly" \
  --privileged --group-add "$group" -v /var/run/docker.sock:/var/run/docker.sock "$image" >/dev/null
else
 sudo docker run -d --name "$container" --restart unless-stopped --env-file "$env_file" \
  -p "$api_port:8080" -p "$p2p_port:9000" \
  --mount "type=bind,source=$data,target=/data" \
  --mount "type=bind,source=$scanner,target=/scanner,readonly" "$image" >/dev/null
fi
started=true
deadline=$((SECONDS+360))
until (( SECONDS >= deadline )); do
 if curl -fsS --max-time 5 "$api/v1/public/health" >/dev/null 2>&1 &&
    curl -fsS --max-time 5 "$api/metrics" | grep -Fq "profile_digest=\"$new_profile\"" &&
    curl -fsS --max-time 5 "$api/metrics" | grep -Fq 'creg_scanner_osv_lookups 5'; then break; fi
 sleep 2
done
[[ $(sudo docker inspect "$container" --format '{{.Image}}') == "$image_id" ]]
[[ $(sudo docker inspect "$preserved" --format '{{.State.Status}}') == exited ]]
sudo grep -Fxq "CREG_OSV_SNAPSHOT_EPOCH=$new_epoch" "$env_file"
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$new_profile" "$env_file"
sudo grep -Fq "\"osv_epoch\": \"$new_epoch\"" "$scanner/manifest.json"
sudo grep -Fq "\"epoch\": \"$new_epoch\"" "$scanner/osv_snapshot.json"
sudo python3 - "$api" "$slot" "$role" "$dead" "$new_epoch" "$new_profile" "$image_id" "$registry_ref" "$identity" "$evidence" "$chain" "$genesis" "$tip" <<'PY'
import hashlib,json,pathlib,re,subprocess,sys,time,urllib.request
api,slot,role,dead,epoch,profile,image,registry,identity,evidence,chain,genesis,tip=sys.argv[1:]
dead=int(dead)
def get(path):
 with urllib.request.urlopen(api+path,timeout=8) as r:return r.read().decode()
def metric(text,name):
 m=re.search(r'^'+re.escape(name)+r'(?:\{[^}]*\})?\s+([0-9.]+)$',text,re.M)
 return float(m.group(1)) if m else None
last={}; deadline=time.time()+360
while time.time()<deadline:
 try:
  stats=json.loads(get('/v1/public/chain/stats'));health=json.loads(get('/v1/public/health'))
  text=get('/metrics');rounds=json.loads(get('/v1/consensus/pbft')).get('rounds',[])
  actual=subprocess.check_output(['sudo','docker','inspect','creg-phase-c-r7-'+slot,'--format','{{.Image}}'],text=True).strip()
  repo=subprocess.check_output(['sudo','docker','image','inspect',actual,'--format','{{join .RepoDigests "\\n"}}'],text=True).splitlines()
  binary=subprocess.check_output(['sudo','docker','exec','creg-phase-c-r7-'+slot,'sha256sum','/app/creg-node'],text=True).split()[0]
  queue={name:metric(text,name) for name in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_durable_finalized_work_dead_letter_depth','creg_forced_inclusion_pending')}
  checks={
   'image_and_registry':actual==image and (registry in repo or ('creg-node@'+registry.rsplit('@',1)[-1]) in repo),
   'node_binary_unchanged':binary=='c7ef1d3e8bb9b34e2e0e4e255bf47f751ecaada4d0617f9ac5a4dbedb262a5b1',
   'signed_chain_and_genesis':stats.get('chain_id')==chain and stats.get('spec_signature_verified') is True and stats.get('genesis_hash','').removeprefix('0x')==genesis,
   'accepted_h12':stats.get('tip_height')==12 and stats.get('finalized_height')==12 and stats.get('tip_hash')==tip and stats.get('package_count')==4,
   'four_validators':stats.get('validator_count')==4 and stats.get('active_validators')==4,
   'role_and_peers':health.get('is_validator') is (role=='validator') and stats.get('peer_count',0)>0,
   'scanner_profile':metric(text,'creg_scanner_profile_ready')==1 and ('profile_digest="'+profile+'"') in text,
   'scanner_inputs_ready':metric(text,'creg_scanner_osv_ready')==1 and metric(text,'creg_scanner_osv_lookups')==5 and metric(text,'creg_scanner_yara_ready')==1 and metric(text,'creg_scanner_threat_intel_ready')==1 and metric(text,'creg_scanner_threat_intel_entries')==100,
   'queues_clear':all(queue[k]==0 for k in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')),
   'dead_letters_unchanged':queue['creg_durable_finalized_work_dead_letter_depth']==dead,
   'pbft_clear':all(r.get('phase') in ('finalised','failed') for r in rounds),
   'p2p_identity_unchanged':hashlib.sha256(pathlib.Path('/var/lib/creg/phase-c-r7/data/p2p/identity.key').read_bytes()).hexdigest()==identity}
  if role=='validator':
   box=health.get('sandbox',{})
   checks['validator_sandbox']=box.get('engine')=='nsjail' and box.get('isolated') is True and box.get('dev_bypass') is False
   checks['signer_journal_present']=pathlib.Path('/var/lib/creg/phase-c-r7/data/pbft-safety.rocksdb').is_dir()
  else:
   checks['validator_signer_journal_not_applicable']=True
  last={'checks':checks,'passed':all(checks.values()),'stats':stats,'queues':queue,'image_id':actual,'registry_digest':registry,'repo_digests':repo,'node_binary_sha256':binary,'pbft_rounds':rounds}
  if last['passed']:break
 except Exception as error:last={'error':str(error)}
 time.sleep(2)
last.update({'schema':'creg-real-r22-scanner-profile-rollout-v1','slot':slot,'role':role,'scanner_epoch':epoch,'scanner_profile_digest':profile,'osv_snapshot_sha256':'452b326cc8d7bea568f21e2fd0d42d9b45503f481a8efe26a174ecb09cabe930','manifest_sha256':'bbaca082dbcece3fd4ef808b17057a27e7862135921bf694a1d2ff458518d932','preserved_container':'creg-phase-c-r7-'+slot+'-r22-before-osv-r11','public_cutover':False,'captured_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())})
pathlib.Path(evidence).parent.mkdir(parents=True,exist_ok=True)
pathlib.Path(evidence).write_text(json.dumps(last,indent=2)+'\n')
print(json.dumps({'slot':slot,'passed':last.get('passed',False),'failed_checks':[k for k,v in last.get('checks',{}).items() if v is not True],'evidence':evidence}))
if not last.get('passed'):raise SystemExit('scanner profile failed readiness; current and preserved containers remain inspectable')
PY
[[ $(sudo docker inspect "$preserved" --format '{{.State.Status}}') == exited ]]
echo "Scanner profile updated on $slot; old container, scanner inputs, data and signer journal are preserved."
