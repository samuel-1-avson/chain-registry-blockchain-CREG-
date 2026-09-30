#!/usr/bin/env bash
set -Eeuo pipefail

container='creg-phase-c-r7-v4'
data='/var/lib/creg/phase-c-r7/data'
scanner='/var/lib/creg/phase-c-r7/scanner-r11-osv-kleur'
env_path='/var/lib/creg/phase-c-r7/node.env'
image_id='sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9'
repo_digest='us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
profile='f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'
chain_id='creg-phase-c-testnet-2026-09-26-r7'
network_genesis='1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234'
tip_hash='8448cce76d7258f051cf9632d2ef7181da1321f5d19a3b5d00f00f8975aa2993'
capture_height=14
package='npm:kleur@4.1.5'
group_id='111'
run_id="$(date -u +%Y%m%dT%H%M%SZ)"
preserved="${container}-w3-original-${run_id}"
failed="${container}-w3-arm-failed-${run_id}"
capture_env="/var/lib/creg/phase-c-r7/node.env.w3-v4-capture-${run_id}"
report="/var/lib/creg/phase-c-r7/w3-v4-arm-${run_id}.json"
mutation_started=0

rollback() {
  rc=$?
  trap - EXIT
  if [[ "$rc" -ne 0 && "$mutation_started" -eq 1 ]]; then
    docker stop --time 20 "$container" >/dev/null 2>&1 || true
    if docker container inspect "$container" >/dev/null 2>&1; then
      docker rename "$container" "$failed" >/dev/null 2>&1 || true
    fi
    if docker container inspect "$preserved" >/dev/null 2>&1 && ! docker container inspect "$container" >/dev/null 2>&1; then
      docker rename "$preserved" "$container"
      docker start "$container" >/dev/null
    fi
    printf 'V4 arm failed; R22 original container restore was attempted. Preserve %s, %s and %s for diagnosis.\n' "$preserved" "$failed" "$capture_env" >&2
  fi
  exit "$rc"
}
trap rollback EXIT

[[ "$EUID" -eq 0 ]] || { echo 'Run this helper as root on the V4 host.' >&2; exit 2; }
[[ -d "$data/chain.rocksdb" && -d "$data/pbft-safety.rocksdb" && -f "$data/p2p/identity.key" ]] || { echo 'V4 chain store, signer journal or P2P identity is missing.' >&2; exit 1; }
[[ -d "$scanner" && -f "$env_path" ]] || { echo 'V4 R11 scanner bundle or node environment file is missing.' >&2; exit 1; }
! docker container inspect "$preserved" >/dev/null 2>&1 || { echo 'Preserved-container name already exists; refusing to overwrite it.' >&2; exit 1; }
! docker container inspect "$failed" >/dev/null 2>&1 || { echo 'Failed-container name already exists; refusing to overwrite it.' >&2; exit 1; }
[[ ! -e "$capture_env" ]] || { echo 'Capture environment file already exists; refusing to overwrite it.' >&2; exit 1; }

python3 - "$container" "$env_path" "$image_id" "$repo_digest" "$profile" "$chain_id" "$network_genesis" "$tip_hash" "$data" "$scanner" <<'PY'
import json, pathlib, re, subprocess, sys, urllib.parse, urllib.request
name, env_path, image_id, repo_digest, profile, chain_id, network_genesis, tip_hash, data, scanner = sys.argv[1:]
def run(args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout.strip()
def get(path):
    with urllib.request.urlopen('http://127.0.0.1:8080' + path, timeout=12) as response:
        return json.loads(response.read().decode())
info = json.loads(run(['docker', 'inspect', name]))[0]
image = json.loads(run(['docker', 'image', 'inspect', image_id]))[0]
raw = pathlib.Path(env_path).read_bytes()
file_env = {}
for line in raw.decode().splitlines():
    if '=' in line and not line.lstrip().startswith('#'):
        key, value = line.split('=', 1)
        file_env[key] = value
container_env = {}
for line in info.get('Config', {}).get('Env', []):
    if '=' in line:
        key, value = line.split('=', 1)
        container_env[key] = value
checks = {
    'container_running': info.get('State', {}).get('Running') is True,
    'r22_image_id_matches': info.get('Image') == image_id,
    'reviewed_registry_digest_present': repo_digest in image.get('RepoDigests', []),
    'env_file_matches_container_config': all(container_env.get(k) == v for k, v in file_env.items()),
    'capture_arm_unset_in_env_file': not any(k.startswith('CREG_PHASE_C_COMMIT_CAPTURE_') for k in file_env),
    'capture_arm_unset_in_container': not any(k.startswith('CREG_PHASE_C_COMMIT_CAPTURE_') for k in container_env),
    'identity_and_profile_match': file_env.get('CREG_NODE_ID') == 'validator-4' and file_env.get('CREG_NODE_ROLE') == 'validator' and file_env.get('CREG_SCANNER_PROFILE_DIGEST') == profile,
    'quorum_safety_matches': file_env.get('CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM', '').lower() == 'false',
    'required_mounts_match': any(m.get('Source') == data and m.get('Destination') == '/data' and m.get('RW') is True for m in info.get('Mounts', [])) and any(m.get('Source') == scanner and m.get('Destination') == '/scanner' and m.get('RW') is False for m in info.get('Mounts', [])),
    'r7_chain_environment_matches': file_env.get('CREG_CHAIN_ID') == chain_id and file_env.get('CREG_GENESIS_HASH', '').removeprefix('0x').lower() == network_genesis,
}
stats = get('/v1/public/chain/stats')
health = get('/v1/public/health')
pbft = get('/v1/consensus/pbft')
package = get('/v1/public/packages/' + urllib.parse.quote('npm:kleur@4.1.5', safe=''))
with urllib.request.urlopen('http://127.0.0.1:8080/metrics', timeout=12) as response:
    metrics = response.read().decode()
checks.update({
    'tip_is_h13': stats.get('tip_height') == 13 and stats.get('finalized_height') == 13 and stats.get('tip_hash') == tip_hash,
    'chain_signature_and_roster': stats.get('chain_id') == chain_id and stats.get('spec_signature_verified') is True and stats.get('active_validators') == 4 and stats.get('validator_count') == 4 and stats.get('peer_count', 0) > 0,
    'node_health_matches_v4': health.get('status') == 'ok' and health.get('node_id') == 'validator-4' and health.get('is_validator') is True,
    'pbft_clear': pbft.get('active_round_count') == 0 and not pbft.get('rounds'),
    'scanner_profile_ready': f'profile_digest="{profile}"' in metrics and re.search(r'^creg_scanner_profile_ready 1\s*$', metrics, re.M) is not None and re.search(r'^creg_scanner_osv_lookups 5\s*$', metrics, re.M) is not None,
    'package_still_verified': package.get('status', '').lower() == 'verified' and package.get('block_phase') == 'committed',
})
if not all(checks.values()):
    raise SystemExit('V4 arm preflight failed: ' + json.dumps({key: value for key, value in checks.items() if not value}))
print(json.dumps({'container_id': info.get('Id'), 'image_id': info.get('Image'), 'restart_policy': info.get('HostConfig', {}).get('RestartPolicy', {}).get('Name'), 'pre_arm_stats': stats, 'checks': checks}))
PY

identity_before="$(sha256sum "$data/p2p/identity.key" | cut -d ' ' -f1)"
env_before="$(sha256sum "$env_path" | cut -d ' ' -f1)"
python3 - "$env_path" "$capture_env" "$chain_id" "$network_genesis" "$capture_height" "$package" <<'PY'
import os, pathlib, sys
source, destination, chain_id, genesis, height, package = sys.argv[1:]
raw = pathlib.Path(source).read_bytes()
for line in raw.decode().splitlines():
    if line.startswith('CREG_PHASE_C_COMMIT_CAPTURE_'):
        raise SystemExit('The original V4 environment already contains a commit-capture arm')
values = {
    'CREG_PHASE_C_COMMIT_CAPTURE_NODE_ID': 'validator-4',
    'CREG_PHASE_C_COMMIT_CAPTURE_CHAIN_ID': chain_id,
    'CREG_PHASE_C_COMMIT_CAPTURE_GENESIS_HASH': genesis,
    'CREG_PHASE_C_COMMIT_CAPTURE_HEIGHT': height,
    'CREG_PHASE_C_COMMIT_CAPTURE_PACKAGE_CANONICAL': package,
    'CREG_PHASE_C_COMMIT_CAPTURE_PAUSE_MS': '30000',
}
payload = raw + (b'' if raw.endswith(b'\n') else b'\n') + ''.join(f'{key}={value}\n' for key, value in values.items()).encode()
fd = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
try:
    os.write(fd, payload)
    os.fsync(fd)
finally:
    os.close(fd)
PY

mutation_started=1
docker stop --time 20 "$container" >/dev/null
[[ "$(docker inspect "$container" --format '{{.State.Status}}')" == 'exited' ]]
docker rename "$container" "$preserved"
docker run -d --name "$container" --restart unless-stopped --env-file "$capture_env" \
  -p 8080:8080 -p 9000:9000 \
  --mount "type=bind,source=$data,target=/data" \
  --mount "type=bind,source=$scanner,target=/scanner,readonly" \
  --privileged --group-add "$group_id" -v /var/run/docker.sock:/var/run/docker.sock \
  "$image_id" >/dev/null

postflight="$(python3 - "$container" "$capture_env" "$image_id" "$repo_digest" "$profile" "$chain_id" "$network_genesis" "$tip_hash" "$data" "$scanner" "$package" <<'PY'
import json, pathlib, re, subprocess, sys, time, urllib.parse, urllib.request
name, env_path, image_id, repo_digest, profile, chain_id, network_genesis, tip_hash, data, scanner, canonical = sys.argv[1:]
def run(args): return subprocess.run(args, check=True, capture_output=True, text=True).stdout.strip()
def json_url(path):
    with urllib.request.urlopen('http://127.0.0.1:8080' + path, timeout=8) as response: return json.loads(response.read().decode())
deadline = time.time() + 240
last_error = None
while time.time() < deadline:
    try:
        info = json.loads(run(['docker', 'inspect', name]))[0]
        stats = json_url('/v1/public/chain/stats')
        health = json_url('/v1/public/health')
        pbft = json_url('/v1/consensus/pbft')
        package = json_url('/v1/public/packages/' + urllib.parse.quote(canonical, safe=''))
        with urllib.request.urlopen('http://127.0.0.1:8080/metrics', timeout=8) as response: metrics = response.read().decode()
        env = {}
        for entry in info.get('Config', {}).get('Env', []):
            if '=' in entry:
                key, value = entry.split('=', 1); env[key] = value
        image = json.loads(run(['docker', 'image', 'inspect', info.get('Image', '')]))[0]
        mounts = [{'source': m.get('Source'), 'destination': m.get('Destination'), 'rw': m.get('RW')} for m in info.get('Mounts', [])]
        checks = {
            'container_running': info.get('State', {}).get('Running') is True,
            'same_reviewed_r22_image': info.get('Image') == image_id and repo_digest in image.get('RepoDigests', []),
            'same_persistent_data_and_read_only_scanner': any(m['source'] == data and m['destination'] == '/data' and m['rw'] is True for m in mounts) and any(m['source'] == scanner and m['destination'] == '/scanner' and m['rw'] is False for m in mounts),
            'capture_arm_exact': env.get('CREG_PHASE_C_COMMIT_CAPTURE_NODE_ID') == 'validator-4' and env.get('CREG_PHASE_C_COMMIT_CAPTURE_CHAIN_ID') == chain_id and env.get('CREG_PHASE_C_COMMIT_CAPTURE_GENESIS_HASH') == network_genesis and env.get('CREG_PHASE_C_COMMIT_CAPTURE_HEIGHT') == '14' and env.get('CREG_PHASE_C_COMMIT_CAPTURE_PACKAGE_CANONICAL') == canonical and env.get('CREG_PHASE_C_COMMIT_CAPTURE_PAUSE_MS') == '30000',
            'same_h13_tip': stats.get('chain_id') == chain_id and stats.get('tip_height') == 13 and stats.get('finalized_height') == 13 and stats.get('tip_hash') == tip_hash and stats.get('spec_signature_verified') is True,
            'v4_live_with_four_validator_roster': health.get('status') == 'ok' and health.get('node_id') == 'validator-4' and health.get('is_validator') is True and stats.get('active_validators') == 4 and stats.get('validator_count') == 4 and stats.get('peer_count', 0) > 0,
            'scanner_profile_ready': f'profile_digest="{profile}"' in metrics and re.search(r'^creg_scanner_profile_ready 1\s*$', metrics, re.M) is not None and re.search(r'^creg_scanner_osv_lookups 5\s*$', metrics, re.M) is not None,
            'pbft_clear': pbft.get('active_round_count') == 0 and not pbft.get('rounds'),
            'package_verified': package.get('status', '').lower() == 'verified' and package.get('block_phase') == 'committed',
        }
        if all(checks.values()):
            print(json.dumps({'container_id': info.get('Id'), 'restart_count': info.get('RestartCount'), 'image_id': info.get('Image'), 'stats': stats, 'checks': checks, 'mounts': mounts}))
            break
        last_error = json.dumps({key: value for key, value in checks.items() if not value})
    except Exception as error:
        last_error = str(error)
    time.sleep(0.5)
else:
    raise SystemExit('Armed V4 did not pass startup readiness within four minutes: ' + str(last_error))
PY
)"

[[ "$(sha256sum "$data/p2p/identity.key" | cut -d ' ' -f1)" == "$identity_before" ]]
[[ "$(sha256sum "$env_path" | cut -d ' ' -f1)" == "$env_before" ]]
[[ "$(docker inspect "$preserved" --format '{{.State.Status}}')" == 'exited' ]]
printf '%s\n' "$postflight" >"$report"
chmod 600 "$report"
cat "$report"
mutation_started=0
trap - EXIT
