#!/usr/bin/env bash
# Replace one stopped Phase C R7 container with the pinned R21 image.
# The chain database, node identity, scanner bundle, environment, and signer
# journal remain in place; the prior container is preserved for rollback.
set -euo pipefail

slot=${1:?slot required}
role=${2:?role required}
api_port=${3:?host API port required}
p2p_port=${4:?host P2P port required}
image_id=${5:?image ID required}
image_tag=creg-node:phase-c-secure-r21-revoke-retirement
expected_index=${6:?manifest-list digest required}
expected_config=${7:?config digest required}
expected_manifest=${8:?platform manifest digest required}
profile=d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4
expected_tip=e0b40d52f7626f46a0bcf85040ec2c34dc1d576af0b1e7d65396d31c325f0837
expected_genesis=0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4
container=creg-phase-c-r7-$slot
preserved=$container-r20-before-r21
failed=$container-r21-failed
base=/var/lib/creg/phase-c-r7
data=$base/data
scanner=$base/scanner-r18-osv-r10
env_path=$base/node.env
env_backup=$base/node.env.r20-preserved-for-r21
api=http://127.0.0.1:$api_port
mutation_started=false

case "$slot:$role" in
  v[2-4]:validator|o[1-4]:observer) ;;
  *) echo 'Invalid R7 slot or role' >&2; exit 2 ;;
esac
case "$api_port:$p2p_port" in
  8080:9000|18080:9001) ;;
  *) echo 'Unexpected R7 port mapping' >&2; exit 2 ;;
esac
for digest in "$image_id" "$expected_index" "$expected_config" "$expected_manifest"; do
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo 'Invalid pinned image digest' >&2; exit 2; }
done

restore_prior() {
  status=$?
  if (( status != 0 )) && [[ "$mutation_started" == true ]]; then
    if sudo docker container inspect "$container" >/dev/null 2>&1; then
      sudo docker stop --time 20 "$container" >/dev/null 2>&1 || true
      if ! sudo docker container inspect "$failed" >/dev/null 2>&1; then
        sudo docker rename "$container" "$failed" || true
      fi
    fi
    if sudo docker container inspect "$preserved" >/dev/null 2>&1 &&
       ! sudo docker container inspect "$container" >/dev/null 2>&1; then
      sudo docker rename "$preserved" "$container" || true
      sudo docker start "$container" >/dev/null 2>&1 || true
    elif sudo docker container inspect "$container" >/dev/null 2>&1 &&
         [[ $(sudo docker inspect "$container" --format '{{.State.Status}}') == exited ]]; then
      sudo docker start "$container" >/dev/null 2>&1 || true
    fi
    echo 'R21 rollout failed; prior container and persistent data were preserved and the prior container was restored where possible.' >&2
  fi
  exit "$status"
}
trap restore_prior EXIT

actual_image=$(sudo docker image inspect "$image_tag" --format '{{.Id}}')
case "$actual_image" in
  "$expected_index"|"$expected_config"|"$expected_manifest") ;;
  *) echo "Loaded R21 image mismatch: $actual_image" >&2; exit 1 ;;
esac
[[ "$actual_image" == "$image_id" || "$image_id" == "$expected_index" || "$image_id" == "$expected_config" || "$image_id" == "$expected_manifest" ]]
for path in "$env_path" "$data/p2p/identity.key" "$data/chain.rocksdb" "$scanner"; do
  sudo test -e "$path" || { echo "Required preserved path missing: $path" >&2; exit 1; }
done
if [[ "$role" == validator ]]; then sudo test -d "$data/pbft-safety.rocksdb"; fi
old_state=$(sudo docker inspect "$container" --format '{{.State.Status}}')
case "$old_state" in running|exited) ;; *) echo "Unexpected prior container state: $old_state" >&2; exit 1 ;; esac
for path in "$preserved" "$failed" "$env_backup"; do
  if sudo test -e "$path" || sudo docker container inspect "$path" >/dev/null 2>&1; then
    echo "Preservation path already exists: $path" >&2; exit 1
  fi
done
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$profile" "$env_path"
sudo grep -Fxq "CREG_NODE_ROLE=$role" "$env_path"
sudo grep -Fxq 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$env_path"
identity_before=$(sudo sha256sum "$data/p2p/identity.key" | cut -d ' ' -f1)
env_before=$(sudo sha256sum "$env_path" | cut -d ' ' -f1)
mutation_started=true
sudo cp -p "$env_path" "$env_backup"
if [[ "$old_state" == running ]]; then sudo docker stop --time 20 "$container" >/dev/null; fi
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
sudo docker "${args[@]}" "$image_tag" >/dev/null
container_image=$(sudo docker inspect "$container" --format '{{.Image}}')
case "$container_image" in
  "$expected_index"|"$expected_config"|"$expected_manifest") ;;
  *) echo "Running container image mismatch: $container_image" >&2; exit 1 ;;
esac
[[ $(sudo sha256sum "$data/p2p/identity.key" | cut -d ' ' -f1) == "$identity_before" ]]
[[ $(sudo sha256sum "$env_path" | cut -d ' ' -f1) == "$env_before" ]]
[[ $(sudo docker inspect "$preserved" --format '{{.State.Status}}') == exited ]]

python3 - "$api" "$role" "$profile" "$expected_tip" "$expected_genesis" <<'PY'
import json, re, sys, time, urllib.parse, urllib.request
api, role, profile, expected_tip, expected_genesis = sys.argv[1:]
def read(path):
    with urllib.request.urlopen(api + path, timeout=8) as response:
        return json.loads(response.read().decode())
def metrics():
    with urllib.request.urlopen(api + '/metrics', timeout=8) as response:
        return response.read().decode()
def metric(text, name):
    found = re.search(r'^' + re.escape(name) + r'\s+([0-9.]+)\s*$', text, re.M)
    return float(found.group(1)) if found else None
deadline = time.time() + 360
last = None
while time.time() < deadline:
    try:
        stats = read('/v1/public/chain/stats')
        health = read('/v1/public/health')
        pbft = read('/v1/consensus/pbft')
        m = metrics()
        package = read('/v1/public/packages/' + urllib.parse.quote('npm:toidentifier@1.0.1', safe=''))
        values = {name: metric(m, name) for name in (
            'creg_pending_block_txs', 'creg_durable_finalized_work_depth',
            'creg_durable_finalized_work_ready_depth', 'creg_forced_inclusion_pending')}
        unresolved = [r for r in pbft.get('rounds', []) if r.get('phase') not in ('finalised', 'failed') or
                      (r.get('phase') == 'failed' and r.get('prepare_count', 0) >= r.get('quorum', 3))]
        checks = {
            'chain_identity': stats.get('chain_id') == 'creg-phase-c-testnet-2026-09-26-r7' and stats.get('spec_signature_verified') is True,
            'genesis_preserved': stats.get('genesis_hash', '').removeprefix('0x') == expected_genesis,
            'h9_preserved': stats.get('tip_height') == 9 and stats.get('tip_hash') == expected_tip,
            'package_revocation_committed': package.get('status') == 'revoked' and package.get('block_phase') == 'committed',
            'four_validator_roster': stats.get('active_validators') == 4 and stats.get('validator_count') == 4,
            'connected_peers': stats.get('peer_count', 0) > 0,
            'correct_role': health.get('is_validator') is (role == 'validator'),
            'scanner_profile_ready': metric(m, 'creg_scanner_profile_ready') == 1 and f'profile_digest="{profile}"' in m,
            'pending_and_forced_queues_clear': all(value == 0 for value in values.values()),
            'no_unfinished_pbft_lock': not unresolved,
        }
        if role == 'validator':
            sandbox = health.get('sandbox', {})
            checks['validator_sandbox_isolated'] = sandbox.get('engine') == 'nsjail' and sandbox.get('isolated') is True and sandbox.get('dev_bypass') is False
        last = {'checks': checks, 'stats': stats, 'health': health, 'pbft': pbft, 'queues': values}
        if all(checks.values()):
            print(json.dumps(last))
            raise SystemExit(0)
    except SystemExit:
        raise
    except Exception as error:
        last = {'last_error': str(error)}
    time.sleep(2)
print(json.dumps(last or {'error': 'readiness was not reached'}))
raise SystemExit(1)
PY

[[ $(sudo docker inspect "$container" --format '{{.State.Running}}') == true ]]
[[ $(sudo docker inspect "$preserved" --format '{{.State.Status}}') == exited ]]
mutation_started=false
trap - EXIT
echo "R21 container restored slot=$slot image=$container_image node_identity_preserved=true prior_container=$preserved"
