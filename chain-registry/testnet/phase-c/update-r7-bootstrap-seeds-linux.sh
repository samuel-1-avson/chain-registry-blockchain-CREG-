#!/usr/bin/env bash
# Recreate one signed bootnode with its own peer ID removed from the seed list.
# The data, P2P identity, PBFT journal, scanner, image and role stay pinned.
set -euo pipefail

slot=${1:?slot}
self_peer_id=${2:?self peer id}
old_seed=${3:?exact current CREG_P2P_SEEDS value}
new_seed=${4:?replacement signed external bootnode multiaddr}
container=creg-phase-c-r7-$slot
preserved=$container-r20-self-seed-preserved
failed=$container-r20-self-seed-failed
base=/var/lib/creg/phase-c-r7
data_dir=$base/data
scanner_dir=$base/scanner-r18-osv-r10
env_path=$base/node.env
backup_path=$base/node.env.r20-self-seed-correction-preserved
api=http://127.0.0.1:8080
old_renamed=false

case "$slot:$self_peer_id" in
  v3:12D3KooW9ygmUCoS7BPxkJRxz8trK7nxDwidTJU7DMe8VBqXs8W8)
    role=validator
    expected_seed=/ip4/34.35.191.131/tcp/9000/p2p/12D3KooWAHukfdmmahm5SGiCoTYpGTB8ez2N38eTUUtPXtstJB8B ;;
  o4:12D3KooWAHukfdmmahm5SGiCoTYpGTB8ez2N38eTUUtPXtstJB8B)
    role=observer
    expected_seed=/ip4/34.156.122.19/tcp/9000/p2p/12D3KooW9ygmUCoS7BPxkJRxz8trK7nxDwidTJU7DMe8VBqXs8W8 ;;
  *) echo 'Only the two signed bootnodes are eligible for this correction' >&2; exit 2 ;;
esac
[[ "$new_seed" == "$expected_seed" ]]
[[ "$new_seed" == /ip4/*/tcp/9000/p2p/* ]]
[[ "$new_seed" != *"$self_peer_id"* ]]
[[ -f "$env_path" && -f "$data_dir/p2p/identity.key" && -d "$data_dir/chain.rocksdb" ]]
[[ -d "$scanner_dir" && ! -e "$backup_path" ]]
[[ ! $(sudo docker inspect "$preserved" --format '{{.Id}}' 2>/dev/null || true) ]]
[[ ! $(sudo docker inspect "$failed" --format '{{.Id}}' 2>/dev/null || true) ]]
[[ $(sudo docker inspect "$container" --format '{{.State.Status}}') == running ]]

restore_on_failure() {
    rc=$?
    if (( rc != 0 )); then
        if [[ "$old_renamed" == true ]]; then
            if sudo docker inspect "$container" >/dev/null 2>&1; then
                sudo docker stop --time 20 "$container" >/dev/null 2>&1 || true
                sudo docker rename "$container" "$failed" >/dev/null 2>&1 || true
            fi
            sudo docker rename "$preserved" "$container" >/dev/null 2>&1 || true
            sudo cp -p "$backup_path" "$env_path" 2>/dev/null || true
            sudo docker start "$container" >/dev/null 2>&1 || true
        elif sudo docker inspect "$container" >/dev/null 2>&1; then
            sudo cp -p "$backup_path" "$env_path" 2>/dev/null || true
            current=$(sudo docker inspect "$container" --format '{{.State.Status}}' 2>/dev/null || true)
            if [[ "$current" == exited || "$current" == created ]]; then
                sudo docker start "$container" >/dev/null 2>&1 || true
            fi
        fi
        echo 'Seed update did not pass; prior container/environment restored where possible' >&2
    fi
    trap - EXIT
    exit "$rc"
}
trap restore_on_failure EXIT

before_json=$(curl -fsS "$api/v1/public/chain/stats")
python3 - "$before_json" <<'PY'
import json,sys
s=json.loads(sys.argv[1])
assert s.get('chain_id')=='creg-phase-c-testnet-2026-09-26-r7'
assert s.get('spec_signature_verified') is True
assert s.get('genesis_hash')=='0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
assert s.get('tip_height')==6 and s.get('tip_hash')=='d8814d091c8a50c2c7bfd535b7524d09bfce17b8c63b6b72c33699b8e52f9fb6'
assert s.get('validator_count')==4 and s.get('active_validators')==4
PY

running_seed=$(sudo docker inspect "$container" --format '{{range .Config.Env}}{{println .}}{{end}}' |
    awk -F= '$1=="CREG_P2P_SEEDS" {sub(/^[^=]*=/, ""); print}')
[[ "$running_seed" == "$old_seed" ]]
image_id=$(sudo docker inspect "$container" --format '{{.Image}}')
case "$image_id" in
  sha256:f6ca255a37d86b0f93c2a57aee6233e4ef1bce16df14f0590e23ec6705dd8391|sha256:8c152eb077f14743d1aa83c0224fc779f88997383597872b9db676d774083101) ;;
  *) echo 'Node image is outside the accepted R20 manifest/config pins' >&2; exit 1 ;;
esac
identity_hash=$(sudo sha256sum "$data_dir/p2p/identity.key" | cut -d ' ' -f1)
started_before=$(sudo docker inspect "$container" --format '{{.State.StartedAt}}')
mounts=$(sudo docker inspect "$container" --format '{{range .Mounts}}{{println .Source "|" .Destination "|" .RW}}{{end}}')
grep -Fxq "$data_dir | /data | true" <<<"$mounts"
grep -Fxq "$scanner_dir | /scanner | false" <<<"$mounts"
ports=$(sudo docker inspect "$container" --format '{{json .HostConfig.PortBindings}}')
python3 - "$ports" <<'PY'
import json,sys
p=json.loads(sys.argv[1])
assert p['8080/tcp'][0]['HostPort']=='8080'
assert p['9000/tcp'][0]['HostPort']=='9000'
PY
[[ $(sudo docker inspect "$container" --format '{{.HostConfig.RestartPolicy.Name}}') == unless-stopped ]]
if [[ "$role" == validator ]]; then
    sudo test -d "$data_dir/pbft-safety.rocksdb"
    docker_group=$(stat -c '%g' /var/run/docker.sock)
fi

sudo cp -p "$env_path" "$backup_path"
sudo python3 - "$env_path" "$old_seed" "$new_seed" "$self_peer_id" <<'PY'
import os,sys,tempfile
path,old,new,self_id=sys.argv[1:]
with open(path,'rb') as source: raw=source.read()
lines=raw.decode('utf-8').splitlines(keepends=True)
matches=[i for i,line in enumerate(lines) if line.rstrip('\r\n').startswith('CREG_P2P_SEEDS=')]
if len(matches)!=1: raise SystemExit('Expected exactly one seed setting')
i=matches[0]
current=lines[i].rstrip('\r\n').split('=',1)[1]
if current!=old: raise SystemExit('Environment seed setting changed; no update made')
if self_id in new: raise SystemExit('Replacement contains this node identity')
eol='\r\n' if lines[i].endswith('\r\n') else '\n' if lines[i].endswith('\n') else ''
lines[i]='CREG_P2P_SEEDS='+new+eol
st=os.stat(path); fd,tmp=tempfile.mkstemp(prefix='.node.env.seed-',dir=os.path.dirname(path))
try:
    with os.fdopen(fd,'wb') as target:
        target.write(''.join(lines).encode('utf-8')); target.flush(); os.fsync(target.fileno())
    os.chmod(tmp,st.st_mode & 0o7777); os.chown(tmp,st.st_uid,st.st_gid); os.replace(tmp,path)
    directory=os.open(os.path.dirname(path),os.O_RDONLY)
    try: os.fsync(directory)
    finally: os.close(directory)
except BaseException:
    try: os.unlink(tmp)
    except FileNotFoundError: pass
    raise
PY

restart_epoch=$(date -u +%Y-%m-%dT%H:%M:%SZ)
sudo docker stop --time 20 "$container" >/dev/null
[[ $(sudo docker inspect "$container" --format '{{.State.Status}}') == exited ]]
sudo docker rename "$container" "$preserved"
old_renamed=true
args=(run -d --name "$container" --restart unless-stopped --env-file "$env_path"
  -p 8080:8080 -p 9000:9000
  --mount "type=bind,source=$data_dir,target=/data"
  --mount "type=bind,source=$scanner_dir,target=/scanner,readonly")
if [[ "$role" == validator ]]; then
    args+=(--privileged --group-add "$docker_group" -v /var/run/docker.sock:/var/run/docker.sock)
fi
sudo docker "${args[@]}" "$image_id" >/dev/null

deadline=$((SECONDS+180)); after_json=''
while (( SECONDS < deadline )); do
    if [[ $(sudo docker inspect "$container" --format '{{.State.Status}}' 2>/dev/null || true) == running ]]; then
        after_json=$(curl -fsS "$api/v1/public/chain/stats" 2>/dev/null || true)
        if [[ -n "$after_json" ]] && python3 - "$after_json" <<'PY' >/dev/null 2>&1
import json,sys
s=json.loads(sys.argv[1])
assert s.get('chain_id')=='creg-phase-c-testnet-2026-09-26-r7' and s.get('spec_signature_verified') is True
assert s.get('tip_height')==6 and s.get('tip_hash')=='d8814d091c8a50c2c7bfd535b7524d09bfce17b8c63b6b72c33699b8e52f9fb6'
assert s.get('validator_count')==4 and s.get('active_validators')==4 and s.get('peer_count',0)>0
PY
        then break; fi
    fi
    sleep 2
done
[[ -n "$after_json" ]]
sleep 25

new_running_seed=$(sudo docker inspect "$container" --format '{{range .Config.Env}}{{println .}}{{end}}' |
    awk -F= '$1=="CREG_P2P_SEEDS" {sub(/^[^=]*=/, ""); print}')
[[ "$new_running_seed" == "$new_seed" ]]
[[ $(sudo docker inspect "$container" --format '{{.Image}}') == "$image_id" ]]
[[ $(sudo docker inspect "$container" --format '{{.State.Status}}') == running ]]
[[ $(sudo docker inspect "$container" --format '{{.State.Health.Status}}') == healthy ]]
[[ $(sudo sha256sum "$data_dir/p2p/identity.key" | cut -d ' ' -f1) == "$identity_hash" ]]
started_after=$(sudo docker inspect "$container" --format '{{.State.StartedAt}}')
[[ "$started_after" != "$started_before" ]]
if [[ "$role" == validator ]]; then sudo test -d "$data_dir/pbft-safety.rocksdb"; fi
self_dial_errors=$(sudo docker logs --since "$restart_epoch" "$container" 2>&1 |
    grep -c 'tried to dial local peer id' || true)
[[ "$self_dial_errors" == 0 ]]

python3 - "$slot" "$role" "$image_id" "$old_seed" "$new_seed" "$before_json" "$after_json" "$started_before" "$started_after" "$self_dial_errors" <<'PY'
import datetime,json,sys
slot,role,image,old,new,before,after,started_before,started_after,self_errors=sys.argv[1:]
b=json.loads(before); a=json.loads(after)
record={"schema":"creg-r7-self-seed-correction-v1","captured_at_utc":datetime.datetime.now(datetime.timezone.utc).isoformat(),
 "slot":slot,"role":role,"image_id":image,"old_seed":old,"new_seed":new,"before_tip_height":b["tip_height"],
 "after_tip_height":a["tip_height"],"after_tip_hash":a["tip_hash"],"chain_id":a["chain_id"],
 "active_validators":a["active_validators"],"peer_count":a["peer_count"],"started_before":started_before,
 "started_after":started_after,"self_dial_errors_since_restart":int(self_errors),"public_cutover":False,
 "passed":a["tip_height"]==6 and a["tip_hash"]=="d8814d091c8a50c2c7bfd535b7524d09bfce17b8c63b6b72c33699b8e52f9fb6" and
          a["chain_id"]=="creg-phase-c-testnet-2026-09-26-r7" and a["active_validators"]==4 and
          a["peer_count"]>0 and started_before!=started_after and int(self_errors)==0}
print(json.dumps(record,separators=(',',':')))
if not record["passed"]: raise SystemExit(1)
PY

trap - EXIT
