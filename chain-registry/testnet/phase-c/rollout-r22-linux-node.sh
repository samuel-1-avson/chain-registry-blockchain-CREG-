#!/usr/bin/env bash
# Replace one real R21 R7 node with the pinned R22 image. Persistent data,
# identity, environment and signer journal remain on the host. The caller must
# run a fresh all-eight readiness capture before each validator replacement.
set -euo pipefail

slot=${1:?slot required}
role=${2:?role required}
api_port=${3:?host API port required}
p2p_port=${4:?host P2P port required}
expected_node_id=${5:?expected node id required}
expected_dead_letters=${6:?expected dead-letter depth required}
expected_image_id=${7:?expected local image/config id required}
expected_registry_ref=${8:?reviewed private registry digest required}
profile=d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4
chain_id=creg-phase-c-testnet-2026-09-26-r7
genesis=0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4
tip_hash=a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d
image=creg-node:phase-c-secure-r22-commit-capture
container=creg-phase-c-r7-$slot
preserved=$container-r21-before-r22
failed=$container-r22-failed
base=/var/lib/creg/phase-c-r7
data=$base/data
env_path=$base/node.env
scanner=$base/scanner-r18-osv-r10
api=http://127.0.0.1:$api_port
evidence_dir=$base/evidence
evidence_path=$evidence_dir/r22-$slot-rollout.json
mutation_started=false

case "$slot:$role" in
  v[2-4]:validator|o[1-4]:observer) ;;
  *) echo 'Invalid R7 slot or role' >&2; exit 2 ;;
esac
case "$api_port:$p2p_port" in
  18080:9001|8080:9000) ;;
  *) echo 'Unexpected R7 port mapping' >&2; exit 2 ;;
esac
case "$expected_image_id" in
  sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3|sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9) ;;
  *) echo 'Unexpected R22 local image/config identity' >&2; exit 2 ;;
esac
case "$expected_registry_ref" in
  us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3) ;;
  *) echo 'Unexpected R22 private registry digest' >&2; exit 2 ;;
esac

case "$slot" in
  v2) prior_ids=(sha256:374eadc62ee9652ee0869efb22ad02ddb0bd8184e11a752a4b4bb303179d9423);;
  v3|v4|o3|o4) prior_ids=(sha256:d2b83a6525314b17434bc98c5bf05181db4fd6d73c962eabd2b5407968dfa95c);;
  o1|o2) prior_ids=(sha256:374eadc62ee9652ee0869efb22ad02ddb0bd8184e11a752a4b4bb303179d9423);;
esac

[[ "$role" == validator && "$expected_node_id" == "validator-${slot:1}" ]] ||
  [[ "$role" == observer && "$expected_node_id" == "observer-${slot:1}" ]] || {
    echo 'Slot, role and node identity do not match' >&2; exit 2;
  }
[[ "$expected_dead_letters" =~ ^[0-9]+$ ]] || { echo 'Invalid dead-letter depth' >&2; exit 2; }
command -v flock >/dev/null || { echo 'Host flock utility is required for a serialized rollout' >&2; exit 1; }
exec 9>"/tmp/creg-r22-$slot-rollout.lock"
flock -n 9 || { echo 'Another R22 rollout is active for this slot' >&2; exit 1; }

for path in "$env_path" "$data/p2p/identity.key" "$data/chain.rocksdb" "$scanner"; do
  sudo test -e "$path" || { echo "Required preserved path missing: $path" >&2; exit 1; }
done
if [[ "$role" == validator ]]; then sudo test -d "$data/pbft-safety.rocksdb"; fi
for name in "$preserved" "$failed"; do
  if sudo docker container inspect "$name" >/dev/null 2>&1; then echo "Preservation container already exists: $name" >&2; exit 1; fi
done
[[ ! -e "$evidence_path" ]] || { echo 'Remote rollout evidence already exists' >&2; exit 1; }
sudo grep -Fxq "CREG_NODE_ID=$expected_node_id" "$env_path"
sudo grep -Fxq "CREG_NODE_ROLE=$role" "$env_path"
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$profile" "$env_path"
sudo grep -Fxq 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$env_path"
if sudo grep -q '^CREG_PHASE_C_COMMIT_CAPTURE_' "$env_path"; then
  echo 'Ordinary rollout requires the commit-capture arm unset' >&2; exit 1
fi

old_state=$(sudo docker inspect "$container" --format '{{.State.Status}}')
old_image=$(sudo docker inspect "$container" --format '{{.Image}}')
[[ "$old_state" == running ]] || { echo "Prior container is not running: $old_state" >&2; exit 1; }
case "$old_image" in "${prior_ids[@]}") ;; *) echo "Unexpected prior image: $old_image" >&2; exit 1 ;; esac

python3 - "$api" "$role" "$chain_id" "$genesis" "$tip_hash" <<'PY'
import json, sys, urllib.request
api, role, chain_id, genesis, tip_hash = sys.argv[1:]
def get(path):
    with urllib.request.urlopen(api + path, timeout=8) as response:
        return json.loads(response.read().decode())
stats = get('/v1/public/chain/stats')
health = get('/v1/public/health')
if stats.get('chain_id') != chain_id or not stats.get('spec_signature_verified') or stats.get('genesis_hash', '').removeprefix('0x') != genesis:
    raise SystemExit('Prior node chain identity mismatch; rollout cancelled')
if stats.get('tip_height') != 12 or stats.get('tip_hash') != tip_hash or stats.get('finalized_height') != 12:
    raise SystemExit('Prior node tip mismatch; rollout cancelled')
if health.get('is_validator') is not (role == 'validator'):
    raise SystemExit('Prior node role mismatch; rollout cancelled')
PY

identity_before=$(sudo sha256sum "$data/p2p/identity.key" | cut -d ' ' -f1)
env_before=$(sudo sha256sum "$env_path" | cut -d ' ' -f1)
started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)

rollback() {
  status=$?
  if (( status != 0 )) && [[ "$mutation_started" == true ]]; then
    current_image=$(sudo docker inspect "$container" --format '{{.Image}}' 2>/dev/null || true)
    current_state=$(sudo docker inspect "$container" --format '{{.State.Status}}' 2>/dev/null || true)
    if [[ "$current_image" == "$expected_image_id" ]]; then
      if [[ "$current_state" == running ]]; then sudo docker stop --time 20 "$container" >/dev/null 2>&1 || true; fi
      if ! sudo docker container inspect "$failed" >/dev/null 2>&1; then sudo docker rename "$container" "$failed" || true; fi
    elif [[ "$current_image" == "$old_image" && "$current_state" == exited ]]; then
      # R22 never started. Restarting the untouched prior container is safe.
      sudo docker start "$container" >/dev/null 2>&1 || true
    fi
    if sudo docker container inspect "$preserved" >/dev/null 2>&1 && ! sudo docker container inspect "$container" >/dev/null 2>&1; then
      # Once R22 has started, its writes may have advanced durable signer state.
      # Keep both containers stopped for review instead of starting older code
      # against that state. The other three validators remain online.
      if [[ "$current_image" != "$expected_image_id" ]]; then
        sudo docker rename "$preserved" "$container" || true
        sudo docker start "$container" >/dev/null 2>&1 || true
      fi
    fi
    if [[ ! -e "$evidence_path" ]]; then
      sudo python3 - "$evidence_path" "$slot" "$role" "$expected_node_id" "$expected_image_id" "$expected_registry_ref" "$old_image" "$status" "$current_image" "$current_state" <<'PY'
import json, os, sys, time
path, slot, role, node_id, expected, registry_ref, prior, status, current, state = sys.argv[1:]
os.makedirs(os.path.dirname(path), exist_ok=True)
with open(path, 'w', encoding='utf8') as output:
    json.dump({'schema':'creg-real-r22-node-rollout-failure-v1','slot':slot,'role':role,'node_id':node_id,
               'expected_image_id':expected,'registry_digest':registry_ref,'prior_image_id':prior,'exit_status':int(status),
               'current_image_id':current or None,'current_state':state or None,
               'preserved_container':'creg-phase-c-r7-'+slot+'-r21-before-r22',
               'failed_container':'creg-phase-c-r7-'+slot+'-r22-failed',
               'captured_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
               'automatic_signer_state_rollback':'not_performed','public_cutover':False}, output, indent=2)
    output.write('\n')
PY
    fi
    echo 'R22 rollout failed; evidence and image states were preserved. Older validator code was not started after R22 ran.' >&2
  fi
  exit "$status"
}
trap rollback EXIT

[[ $(sudo docker image inspect "$image" --format '{{.Id}}') == "$expected_image_id" ]] || {
  echo 'Loaded R22 image digest mismatch; rollout cancelled' >&2; exit 1;
}
repo_digests=$(sudo docker image inspect "$image" --format '{{join .RepoDigests "\n"}}')
grep -Fxq "$expected_registry_ref" <<<"$repo_digests" || { echo 'Loaded image is missing the reviewed private registry digest' >&2; exit 1; }
mutation_started=true
sudo docker stop --time 20 "$container" >/dev/null
[[ $(sudo docker inspect "$container" --format '{{.State.Status}}') == exited ]]
sudo docker rename "$container" "$preserved"

args=(run -d --name "$container" --restart unless-stopped --env-file "$env_path"
  -p "$api_port:8080" -p "$p2p_port:9000"
  --mount "type=bind,source=$data,target=/data"
  --mount "type=bind,source=$scanner,target=/scanner,readonly")
if [[ "$role" == validator ]]; then
  docker_group=$(stat -c '%g' /var/run/docker.sock)
  args+=(--privileged --group-add "$docker_group" -v /var/run/docker.sock:/var/run/docker.sock)
fi
sudo docker "${args[@]}" "$image" >/dev/null
[[ $(sudo docker inspect "$container" --format '{{.Image}}') == "$expected_image_id" ]]
repo_digests=$(sudo docker image inspect "$image" --format '{{join .RepoDigests "\n"}}')
grep -Fxq "$expected_registry_ref" <<<"$repo_digests"
[[ $(sudo sha256sum "$data/p2p/identity.key" | cut -d ' ' -f1) == "$identity_before" ]]
[[ $(sudo sha256sum "$env_path" | cut -d ' ' -f1) == "$env_before" ]]
[[ $(sudo docker inspect "$preserved" --format '{{.State.Status}}') == exited ]]

sudo python3 - "$api" "$slot" "$role" "$expected_node_id" "$expected_dead_letters" "$expected_image_id" "$expected_registry_ref" "$profile" "$chain_id" "$genesis" "$tip_hash" "$started_at" "$data" "$env_path" "$evidence_path" "$old_image" "$identity_before" "$env_before" <<'PY'
import hashlib, json, os, pathlib, re, subprocess, sys, time, urllib.parse, urllib.request
api, slot, role, expected_node_id, dead_letters, expected_image_id, expected_registry_ref, profile, chain_id, genesis, tip_hash, started_at, data, env_path, evidence_path, old_image, identity_before, env_before = sys.argv[1:]
dead_letters = int(dead_letters)
deadline = time.time() + 360
last = None
def get(path):
    with urllib.request.urlopen(api + path, timeout=8) as response:
        return json.loads(response.read().decode())
def metrics():
    with urllib.request.urlopen(api + '/metrics', timeout=8) as response:
        return response.read().decode()
def metric(text, name):
    match = re.search(r'^' + re.escape(name) + r'(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)$', text, re.M)
    return float(match.group(1)) if match else None
while time.time() < deadline:
    try:
        stats = get('/v1/public/chain/stats')
        health = get('/v1/public/health')
        pbft = get('/v1/consensus/pbft')
        text = metrics()
        unresolved = [r for r in pbft.get('rounds', []) if r.get('phase') not in ('finalised', 'failed') or
                      (r.get('phase') == 'failed' and r.get('prepare_count', 0) >= r.get('quorum', 3))]
        values = {name: metric(text, name) for name in (
            'creg_pending_block_txs', 'creg_durable_finalized_work_depth',
            'creg_durable_finalized_work_ready_depth', 'creg_durable_finalized_work_dead_letter_depth',
            'creg_forced_inclusion_pending')}
        container = 'creg-phase-c-r7-' + slot
        image_id = subprocess.check_output(['sudo', 'docker', 'inspect', container, '--format', '{{.Image}}'], text=True).strip()
        repo_digests = subprocess.check_output(['sudo', 'docker', 'image', 'inspect', image_id, '--format', '{{join .RepoDigests "\\n"}}'], text=True)
        binary_hash = subprocess.check_output(['sudo', 'docker', 'exec', container, 'sha256sum', '/app/creg-node'], text=True).split()[0]
        checks = {
            'image_config_id': image_id == expected_image_id,
            'registry_digest': expected_registry_ref in repo_digests.splitlines(),
            'node_binary_hash': binary_hash == 'c7ef1d3e8bb9b34e2e0e4e255bf47f751ecaada4d0617f9ac5a4dbedb262a5b1',
            'chain_identity': stats.get('chain_id') == chain_id and stats.get('spec_signature_verified') is True,
            'genesis_preserved': stats.get('genesis_hash', '').removeprefix('0x') == genesis,
            'h12_preserved': stats.get('tip_height') == 12 and stats.get('finalized_height') == 12 and stats.get('tip_hash') == tip_hash,
            'four_validator_roster': stats.get('active_validators') == 4 and stats.get('validator_count') == 4,
            'connected_peers': stats.get('peer_count', 0) > 0,
            'correct_role': health.get('is_validator') is (role == 'validator'),
            'scanner_profile_ready': metric(text, 'creg_scanner_profile_ready') == 1 and f'profile_digest="{profile}"' in text,
            'yara_ready': metric(text, 'creg_scanner_yara_ready') == 1,
            'threat_feed_ready': metric(text, 'creg_scanner_threat_intel_ready') == 1 and metric(text, 'creg_scanner_threat_intel_entries') == 100,
            'osv_ready': metric(text, 'creg_scanner_osv_ready') == 1,
            'queues_clear': all(values[k] == 0 for k in ('creg_pending_block_txs', 'creg_durable_finalized_work_depth', 'creg_durable_finalized_work_ready_depth', 'creg_forced_inclusion_pending')),
            'terminal_records_preserved': values['creg_durable_finalized_work_dead_letter_depth'] == dead_letters,
            'pbft_clear': not unresolved,
        }
        if role == 'validator':
            sandbox = health.get('sandbox', {})
            checks['sandbox_isolated'] = sandbox.get('engine') == 'nsjail' and sandbox.get('isolated') is True and sandbox.get('dev_bypass') is False
        last = {'checks': checks, 'stats': stats, 'health': health, 'pbft_clear': not unresolved, 'queues': values}
        if all(checks.values()):
            break
    except Exception as error:
        last = {'error': str(error)}
    time.sleep(2)
passed = bool(last and last.get('checks') and all(last['checks'].values()))
identity_path = pathlib.Path(data) / 'p2p' / 'identity.key'
env_bytes = pathlib.Path(env_path).read_bytes()
logs = subprocess.run(['sudo', 'docker', 'logs', '--since', started_at, 'creg-phase-c-r7-' + slot], capture_output=True, text=True)
marker_lines = [line for line in (logs.stdout + logs.stderr).splitlines() if any(key in line for key in ('pbft_local_commit_durable', 'pbft_local_commit_recovered', 'Recovered durable local PBFT COMMIT'))]
record = {
    'schema': 'creg-real-r22-node-rollout-v1',
    'slot': slot,
    'role': role,
    'node_id': expected_node_id,
    'image_id': expected_image_id,
    'registry_digest': expected_registry_ref,
    'prior_image_id': old_image,
    'completed_at_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
    'p2p_identity_sha256': hashlib.sha256(identity_path.read_bytes()).hexdigest(),
    'node_env_sha256': hashlib.sha256(env_bytes).hexdigest(),
    'p2p_identity_preserved': hashlib.sha256(identity_path.read_bytes()).hexdigest() == identity_before,
    'node_env_preserved': hashlib.sha256(env_bytes).hexdigest() == env_before,
    'chain_store_preserved': (pathlib.Path(data) / 'chain.rocksdb').is_dir(),
    'signer_journal_preserved': role == 'validator' and (pathlib.Path(data) / 'pbft-safety.rocksdb').is_dir(),
    'startup_markers': marker_lines,
    'startup_marker_count': len(marker_lines),
    'passed': passed,
    'result': last,
    'public_cutover': False,
}
os.makedirs(os.path.dirname(evidence_path), exist_ok=True)
with open(evidence_path, 'w', encoding='utf8') as output:
    json.dump(record, output, indent=2)
    output.write('\n')
print(json.dumps(record))
raise SystemExit(0 if passed else 1)
PY
