#!/usr/bin/env bash
set -Eeuo pipefail

container='creg-phase-c-r7-v4'
original='creg-phase-c-r7-v4-w3-original-20260927T073331Z'
data='/var/lib/creg/phase-c-r7/data'
scanner='/var/lib/creg/phase-c-r7/scanner-r11-osv-kleur'
env_path='/var/lib/creg/phase-c-r7/node.env'
image_id='sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9'
repo_digest='us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
profile='f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'
chain_id='creg-phase-c-testnet-2026-09-26-r7'
network_genesis='1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234'
expected_h14='2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'
canonical='npm:kleur@4.1.5'
report='/var/lib/creg/phase-c-r7/w3-v4-disarm-no-match.json'
capture_report='/var/lib/creg/phase-c-r7/w3-v4-durable-commit-capture.json'
run_id="$(date -u +%Y%m%dT%H%M%SZ)"
armed_saved="${container}-w3-armed-no-match-${run_id}"
failed="${container}-w3-disarm-failed-${run_id}"
mutation_started=0

rollback() {
  rc=$?
  trap - EXIT
  if [[ "$rc" -ne 0 && "$mutation_started" -eq 1 ]]; then
    docker stop --time 20 "$container" >/dev/null 2>&1 || true
    if docker container inspect "$container" >/dev/null 2>&1; then
      docker rename "$container" "$failed" >/dev/null 2>&1 || true
    fi
    if docker container inspect "$armed_saved" >/dev/null 2>&1 && ! docker container inspect "$container" >/dev/null 2>&1; then
      docker rename "$armed_saved" "$container"
      docker start "$container" >/dev/null
    fi
    printf 'V4 disarm failed; the armed R22 container restore was attempted. Preserve %s and %s for diagnosis.\n' "$armed_saved" "$failed" >&2
  fi
  exit "$rc"
}
trap rollback EXIT

[[ "$EUID" -eq 0 ]] || { echo 'Run this helper as root on the V4 host.' >&2; exit 2; }
docker container inspect "$container" >/dev/null
docker container inspect "$original" >/dev/null
! docker container inspect "$armed_saved" >/dev/null 2>&1 || { echo 'Armed-container preservation name already exists.' >&2; exit 1; }
! docker container inspect "$failed" >/dev/null 2>&1 || { echo 'Failed-container name already exists.' >&2; exit 1; }

python3 - "$container" "$original" "$env_path" "$image_id" "$repo_digest" "$profile" "$chain_id" "$network_genesis" "$data" "$scanner" "$expected_h14" "$canonical" "$capture_report" <<'PY'
import json, pathlib, re, subprocess, sys, urllib.parse, urllib.request
name, original, env_path, image_id, repo_digest, profile, chain_id, genesis, data, scanner, expected_h14, canonical, report_path = sys.argv[1:]
def run(args, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True).stdout.strip()
def get(path):
    with urllib.request.urlopen('http://127.0.0.1:8080' + path, timeout=12) as response:
        return json.loads(response.read().decode())
def inspect(name):
    return json.loads(run(['docker', 'inspect', name]))[0]
def env_map(container):
    result = {}
    for entry in container.get('Config', {}).get('Env', []):
        if '=' in entry:
            key, value = entry.split('=', 1)
            result[key] = value
    return result
def mounts_match(container):
    mounts = container.get('Mounts', [])
    return any(m.get('Source') == data and m.get('Destination') == '/data' and m.get('RW') is True for m in mounts) and any(m.get('Source') == scanner and m.get('Destination') == '/scanner' and m.get('RW') is False for m in mounts)

armed = inspect(name)
saved = inspect(original)
image = json.loads(run(['docker', 'image', 'inspect', image_id]))[0]
armed_env = env_map(armed)
saved_env = env_map(saved)
source_env = {}
for line in pathlib.Path(env_path).read_text().splitlines():
    if '=' in line and not line.lstrip().startswith('#'):
        key, value = line.split('=', 1)
        source_env[key] = value
stats = get('/v1/public/chain/stats')
health = get('/v1/public/health')
pbft = get('/v1/consensus/pbft')
block = get('/v1/blocks/14')
package = get('/v1/public/packages/' + urllib.parse.quote(canonical, safe=''))
with urllib.request.urlopen('http://127.0.0.1:8080/metrics', timeout=12) as response:
    metrics = response.read().decode()
capture_report = json.loads(pathlib.Path(report_path).read_text())
logs = run(['docker', 'logs', '--since', capture_report.get('armed_at_utc', '10m'), name], check=False)
target_markers = [line for line in logs.splitlines() if ('pbft_local_commit_durable' in line or 'phase_c_durable_commit_capture_paused' in line) and 'height=14' in line]
checks = {
    'armed_r22_container_running': armed.get('Image') == image_id and armed.get('State', {}).get('Running') is True,
    'saved_original_is_stopped_same_r22': saved.get('Image') == image_id and saved.get('State', {}).get('Status') == 'exited',
    'reviewed_registry_digest_present': repo_digest in image.get('RepoDigests', []),
    'both_containers_preserve_the_same_data_and_scanner_mounts': mounts_match(armed) and mounts_match(saved),
    'host_node_env_is_unarmed': not any(key.startswith('CREG_PHASE_C_COMMIT_CAPTURE_') for key in source_env),
    'saved_original_container_env_matches_host_file': all(saved_env.get(key) == value for key, value in source_env.items()),
    'armed_container_has_exact_capture_environment': armed_env.get('CREG_PHASE_C_COMMIT_CAPTURE_NODE_ID') == 'validator-4' and armed_env.get('CREG_PHASE_C_COMMIT_CAPTURE_CHAIN_ID') == chain_id and armed_env.get('CREG_PHASE_C_COMMIT_CAPTURE_GENESIS_HASH') == genesis and armed_env.get('CREG_PHASE_C_COMMIT_CAPTURE_HEIGHT') == '14' and armed_env.get('CREG_PHASE_C_COMMIT_CAPTURE_PACKAGE_CANONICAL') == canonical and armed_env.get('CREG_PHASE_C_COMMIT_CAPTURE_PAUSE_MS') == '30000',
    'monitor_saw_no_durable_or_pause_marker': capture_report.get('durable_marker') is None and capture_report.get('pause_marker') is None and capture_report.get('kill') is None and not target_markers,
    'monitor_is_not_claiming_a_capture': capture_report.get('result') == 'waiting_for_exact_durable_commit_and_pause_markers',
    'h14_is_committed_with_validator_3_as_proposer': block.get('header', {}).get('proposer_id') == 'validator-3' and block.get('hash') == expected_h14 and stats.get('tip_height') == 14 and stats.get('tip_hash') == expected_h14,
    'owner_package_is_revoked': package.get('status', '').lower() == 'revoked' and package.get('block_phase') == 'committed',
    'v4_is_healthy_and_pbft_clear': health.get('status') == 'ok' and health.get('node_id') == 'validator-4' and health.get('is_validator') is True and pbft.get('active_round_count') == 0 and not pbft.get('rounds'),
    'scanner_profile_ready': f'profile_digest="{profile}"' in metrics and re.search(r'^creg_scanner_profile_ready 1\s*$', metrics, re.M) is not None and re.search(r'^creg_scanner_osv_lookups 5\s*$', metrics, re.M) is not None,
    'chain_identity_matches': stats.get('chain_id') == chain_id and stats.get('spec_signature_verified') is True,
}
if not all(checks.values()):
    raise SystemExit('V4 disarm preflight failed: ' + json.dumps({key: value for key, value in checks.items() if not value}))
identity = pathlib.Path(data) / 'p2p' / 'identity.key'
journal = pathlib.Path(data) / 'pbft-safety.rocksdb'
if not identity.is_file() or not journal.is_dir():
    raise SystemExit('V4 persistent P2P identity or PBFT signer journal is missing.')
print(json.dumps({'armed_container_id': armed.get('Id'), 'saved_unarmed_container_id': saved.get('Id'), 'image_id': armed.get('Image'), 'pre_disarm_tip': stats, 'h14_proposer': block.get('header', {}).get('proposer_id'), 'target_markers_found': len(target_markers), 'checks': checks}))
PY

identity_before="$(sha256sum "$data/p2p/identity.key" | cut -d ' ' -f1)"
mutation_started=1
docker stop --time 20 "$container" >/dev/null
[[ "$(docker inspect "$container" --format '{{.State.Status}}')" == 'exited' ]]
docker rename "$container" "$armed_saved"
docker rename "$original" "$container"
docker start "$container" >/dev/null

postflight="$(python3 - "$container" "$env_path" "$image_id" "$repo_digest" "$profile" "$chain_id" "$data" "$scanner" "$expected_h14" "$canonical" <<'PY'
import json, pathlib, re, subprocess, sys, time, urllib.parse, urllib.request
name, env_path, image_id, repo_digest, profile, chain_id, data, scanner, expected_h14, canonical = sys.argv[1:]
def run(args): return subprocess.run(args, check=True, capture_output=True, text=True).stdout.strip()
def get(path):
    with urllib.request.urlopen('http://127.0.0.1:8080' + path, timeout=8) as response: return json.loads(response.read().decode())
deadline = time.monotonic() + 240
last = None
while time.monotonic() < deadline:
    try:
        info = json.loads(run(['docker', 'inspect', name]))[0]
        image = json.loads(run(['docker', 'image', 'inspect', image_id]))[0]
        stats = get('/v1/public/chain/stats')
        health = get('/v1/public/health')
        pbft = get('/v1/consensus/pbft')
        block = get('/v1/blocks/14')
        package = get('/v1/public/packages/' + urllib.parse.quote(canonical, safe=''))
        with urllib.request.urlopen('http://127.0.0.1:8080/metrics', timeout=8) as response: metrics = response.read().decode()
        env = {}
        for item in info.get('Config', {}).get('Env', []):
            if '=' in item:
                key, value = item.split('=', 1); env[key] = value
        mounts = info.get('Mounts', [])
        checks = {
            'container_running': info.get('Image') == image_id and info.get('State', {}).get('Running') is True,
            'reviewed_registry_digest_present': repo_digest in image.get('RepoDigests', []),
            'restart_policy_unless_stopped': info.get('HostConfig', {}).get('RestartPolicy', {}).get('Name') == 'unless-stopped',
            'same_persistent_data_and_read_only_scanner': any(m.get('Source') == data and m.get('Destination') == '/data' and m.get('RW') is True for m in mounts) and any(m.get('Source') == scanner and m.get('Destination') == '/scanner' and m.get('RW') is False for m in mounts),
            'same_h14_canonical_tip': stats.get('chain_id') == chain_id and stats.get('tip_height') == 14 and stats.get('finalized_height') == 14 and stats.get('tip_hash') == expected_h14 and block.get('hash') == expected_h14 and block.get('header', {}).get('proposer_id') == 'validator-3',
            'v4_healthy_and_peered': health.get('status') == 'ok' and health.get('node_id') == 'validator-4' and health.get('is_validator') is True and stats.get('active_validators') == 4 and stats.get('validator_count') == 4 and stats.get('peer_count', 0) > 0,
            'capture_arm_unset': not any(key.startswith('CREG_PHASE_C_COMMIT_CAPTURE_') for key in env),
            'scanner_profile_ready': f'profile_digest="{profile}"' in metrics and re.search(r'^creg_scanner_profile_ready 1\s*$', metrics, re.M) is not None and re.search(r'^creg_scanner_osv_lookups 5\s*$', metrics, re.M) is not None,
            'pbft_clear': pbft.get('active_round_count') == 0 and not pbft.get('rounds'),
            'package_revoked': package.get('status', '').lower() == 'revoked' and package.get('block_phase') == 'committed',
            'signed_spec_verified': stats.get('spec_signature_verified') is True,
        }
        identity = pathlib.Path(data) / 'p2p' / 'identity.key'
        journal = pathlib.Path(data) / 'pbft-safety.rocksdb'
        checks['persistent_identity_and_signer_journal_present'] = identity.is_file() and journal.is_dir()
        if all(checks.values()):
            print(json.dumps({'container_id': info.get('Id'), 'restart_count': info.get('RestartCount'), 'image_id': info.get('Image'), 'stats': stats, 'health': health, 'package_status': package.get('status'), 'checks': checks}))
            break
        last = json.dumps({key: value for key, value in checks.items() if not value})
    except Exception as error:
        last = str(error)
    time.sleep(0.5)
else:
    raise SystemExit('Restored unarmed V4 failed postflight: ' + str(last))
PY
)"

[[ "$(sha256sum "$data/p2p/identity.key" | cut -d ' ' -f1)" == "$identity_before" ]]
[[ "$(docker inspect "$armed_saved" --format '{{.State.Status}}')" == 'exited' ]]
for stale_env in \
  /var/lib/creg/phase-c-r7/node.env.w3-v4-capture-20260927T072510Z \
  /var/lib/creg/phase-c-r7/node.env.w3-v4-capture-20260927T073331Z; do
  if [[ -f "$stale_env" ]]; then rm -f -- "$stale_env"; fi
done
printf '%s\n' "$postflight" >"$report"
chmod 600 "$report"
cat "$report"
mutation_started=0
trap - EXIT
