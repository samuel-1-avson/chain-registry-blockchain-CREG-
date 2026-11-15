#!/usr/bin/env bash
# ADR-016 §11 canary on observer-3: replace only the node image and turn on
# equivocation detection (detection only, no slashing). Same node.env file,
# ports and mounts; the flag is passed with -e so node.env stays byte-for-byte
# unchanged. Backs up the data directory and keeps the r33 container (stopped)
# for rollback.
# Usage: o3-swap-r34.sh NEW_IMAGE_ID EXPECTED_REVISION EXPECTED_BINARY_SHA256
set -euo pipefail
new_image=$1; expected_rev=$2; expected_binary=$3
[[ "$new_image" =~ ^sha256:[0-9a-f]{64}$ && "$expected_rev" =~ ^[0-9a-f]{40}$ && "$expected_binary" =~ ^[0-9a-f]{64}$ ]]
c=creg-phase-c-r7-o3; base=/var/lib/creg/phase-c-r7; data=$base/data; envf=$base/node.env
scanner=$base/scanner-r14-ansi-regex-live
old_image=sha256:dbc881f366152da3edd5988eb5f7a759def3707e58a3b1c3c83642879016eac7
ts=$(date -u +%Y%m%dT%H%M%SZ)
preserved=$c-r33-before-r34; backup=$base/data-before-r34-$ts
evidence=$base/evidence/r34-o3-equivocation-canary-$ts.json; pre=/tmp/r34-o3-pre-$ts.json
api=http://127.0.0.1:8080; started=false
exec 9>/tmp/creg-o3-r34.lock; flock -n 9

echo "== pre-checks"
[[ $(sudo docker inspect "$c" --format '{{.Image}}') == "$old_image" ]]
[[ $(sudo docker inspect "$c" --format '{{.State.Status}}') == running ]]
if sudo docker container inspect "$preserved" >/dev/null 2>&1; then echo "$preserved already exists"; exit 1; fi
[[ $(sudo docker image inspect "$new_image" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}') == "$expected_rev" ]]
[[ $(sudo docker run --rm --entrypoint /usr/bin/sha256sum "$new_image" /app/creg-node | cut -d' ' -f1) == "$expected_binary" ]]
for p in "$envf" "$data/p2p/identity.key" "$data/chain.rocksdb" "$scanner"; do sudo test -e "$p"; done
sudo grep -Fxq 'CREG_NODE_ID=observer-3' "$envf"
sudo grep -Fxq 'CREG_NODE_ROLE=observer' "$envf"
sudo grep -Fxq 'CREG_IS_VALIDATOR=false' "$envf"
if sudo grep -q '^CREG_EQUIVOCATION_DETECTION=' "$envf"; then echo "node.env already sets CREG_EQUIVOCATION_DETECTION"; exit 1; fi
if sudo test -e "$data/equivocation-evidence"; then echo "equivocation-evidence already exists in the data dir"; exit 1; fi
mounts=$(sudo docker inspect "$c" --format '{{range .Mounts}}{{println .Source "|" .Destination "|" .RW}}{{end}}')
grep -Fq "$data | /data | true" <<<"$mounts"
grep -Fq "$scanner | /scanner | false" <<<"$mounts"
identity=$(sudo sha256sum "$data/p2p/identity.key" | cut -d' ' -f1)
env_sha=$(sudo sha256sum "$envf" | cut -d' ' -f1)
python3 - "$api" "$pre" <<'PY'
import json, re, sys, urllib.request
api, out = sys.argv[1:]
def get(p):
    with urllib.request.urlopen(api + p, timeout=8) as r:
        return r.read().decode()
s = json.loads(get('/v1/public/chain/stats')); t = get('/metrics')
assert s.get('chain_id') == 'creg-phase-c-testnet-2026-09-26-r7' and s.get('spec_signature_verified') is True
m = re.search(r'profile_digest="([0-9a-f]{64})"', t)
keep = {k: s.get(k) for k in ('tip_height', 'tip_hash', 'finalized_height', 'package_count', 'genesis_hash')}
keep['profile_digest'] = m.group(1) if m else None
json.dump(keep, open(out, 'w'))
print('pre', keep)
PY

rollback_before_start() {
  status=$?
  if (( status != 0 )) && [[ "$started" == false ]]; then
    echo "pre-start failure; restoring the old container"
    if sudo docker container inspect "$preserved" >/dev/null 2>&1 && ! sudo docker container inspect "$c" >/dev/null 2>&1; then
      sudo docker rename "$preserved" "$c" || true
    fi
    sudo docker start "$c" >/dev/null 2>&1 || true
  fi
  exit "$status"
}
trap rollback_before_start EXIT

echo "== stop, back up, swap"
sudo docker stop --time 20 "$c" >/dev/null
sudo cp -a "$data" "$backup"
sudo diff -r "$data" "$backup" >/dev/null
sudo docker rename "$c" "$preserved"
sudo docker run -d --name "$c" --restart unless-stopped --env-file "$envf" \
  -e CREG_EQUIVOCATION_DETECTION=true \
  -p 8080:8080 -p 9000:9000 \
  --mount "type=bind,source=$data,target=/data" \
  --mount "type=bind,source=$scanner,target=/scanner,readonly" "$new_image" >/dev/null
started=true
echo "started new container; backup at $backup; old container kept as $preserved"

echo "== readiness (up to 6 minutes)"
sudo python3 - "$api" "$c" "$new_image" "$expected_binary" "$identity" "$pre" "$evidence" "$preserved" "$backup" "$expected_rev" "$envf" "$env_sha" <<'PY'
import hashlib, json, pathlib, re, subprocess, sys, time, urllib.request
(api, c, image, binary_expected, identity, pre_path, evidence, preserved, backup, rev,
 envf, env_sha) = sys.argv[1:]
pre = json.load(open(pre_path))
ENABLED_LINE = 'Equivocation detection enabled (detection only, no slashing; ADR-016)'
JOURNAL_WARNING = 'PBFT signing is blocked because its durable safety journal is not initialized'
def get(p):
    with urllib.request.urlopen(api + p, timeout=8) as r:
        return r.read().decode()
def metric(text, name):
    m = re.search(r'^' + re.escape(name) + r'(?:\{[^}]*\})?\s+(-?[0-9.e+]+)$', text, re.M)
    return float(m.group(1)) if m else None
def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
last = {}; deadline = time.time() + 360
while time.time() < deadline:
    try:
        s = json.loads(get('/v1/public/chain/stats')); h = json.loads(get('/v1/public/health')); t = get('/metrics')
        rounds = json.loads(get('/v1/consensus/pbft')).get('rounds', [])
        ev = json.loads(get('/v1/public/evidence/equivocation'))
        actual = subprocess.check_output(['docker', 'inspect', c, '--format', '{{.Image}}'], text=True).strip()
        env = json.loads(subprocess.check_output(['docker', 'inspect', c, '--format', '{{json .Config.Env}}'], text=True))
        binary = subprocess.check_output(['docker', 'exec', c, 'sha256sum', '/app/creg-node'], text=True).split()[0]
        logs = subprocess.run(['docker', 'logs', c], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True).stdout
        profile = re.search(r'profile_digest="([0-9a-f]{64})"', t)
        queues = {n: metric(t, n) for n in ('creg_pending_block_txs', 'creg_durable_finalized_work_depth',
                  'creg_durable_finalized_work_ready_depth', 'creg_durable_finalized_work_dead_letter_depth',
                  'creg_forced_inclusion_pending')}
        checks = {
            'new_image_running': actual == image,
            'node_binary_matches_build': binary == binary_expected,
            'signed_chain_and_genesis': s.get('chain_id') == 'creg-phase-c-testnet-2026-09-26-r7'
                and s.get('spec_signature_verified') is True and s.get('genesis_hash') == pre['genesis_hash'],
            'chain_not_behind': (s.get('tip_height') or 0) >= pre['tip_height']
                and (s.get('finalized_height') or 0) >= pre['finalized_height']
                and (s.get('tip_height') != pre['tip_height'] or s.get('tip_hash') == pre['tip_hash']),
            'packages_kept': (s.get('package_count') or 0) >= pre['package_count'],
            'four_validators': s.get('validator_count') == 4 and s.get('active_validators') == 4,
            'observer_with_peers': h.get('is_validator') is False and (s.get('peer_count') or 0) > 0,
            'scanner_profile_unchanged': metric(t, 'creg_scanner_profile_ready') == 1
                and profile is not None and profile.group(1) == pre['profile_digest'],
            'queues_clear': all(queues[k] == 0 for k in queues),
            'pbft_clear': all(r.get('phase') in ('finalised', 'failed') for r in rounds),
            'p2p_identity_unchanged': sha('/var/lib/creg/phase-c-r7/data/p2p/identity.key') == identity,
            'env_file_unchanged': sha(envf) == env_sha,
            'detection_flag_on_container': 'CREG_EQUIVOCATION_DETECTION=true' in env,
            'detection_startup_log': ENABLED_LINE in logs,
            'detection_metric_enabled': metric(t, 'creg_equivocation_detection_enabled') == 1,
            'no_evidence_metric': metric(t, 'creg_equivocation_evidence') == 0,
            'no_persist_failures': metric(t, 'creg_equivocation_persist_failures_total') == 0,
            'evidence_endpoint_enabled_and_empty': ev.get('detection_enabled') is True
                and ev.get('count') == 0 and ev.get('evidence') == [],
        }
        info = {
            'equivocation_tracked_votes': metric(t, 'creg_equivocation_tracked_votes'),
            'equivocation_pending_votes': metric(t, 'creg_equivocation_pending_votes'),
            'equivocation_votes_rejected_total': metric(t, 'creg_equivocation_votes_rejected_total'),
            'journal_warning_lines_since_start': logs.count(JOURNAL_WARNING),
            'error_lines_since_start': sum(1 for l in logs.splitlines() if ' ERROR ' in l or '"level":"ERROR"' in l),
            'log_lines_since_start': len(logs.splitlines()),
            'validator_set_sync_state_code': metric(t, 'creg_validator_set_sync_state_code'),
            'chain_tip_age_seconds': metric(t, 'creg_chain_tip_age_seconds'),
        }
        last = {'checks': checks, 'passed': all(checks.values()), 'informational': info,
                'stats': {k: s.get(k) for k in ('chain_id', 'tip_height', 'tip_hash', 'finalized_height',
                          'package_count', 'peer_count', 'validator_count', 'active_validators',
                          'spec_signature_verified', 'genesis_hash')},
                'equivocation_endpoint': ev, 'queues': queues,
                'image_id': actual, 'node_binary_sha256': binary}
        if last['passed']:
            break
    except Exception as error:
        last = {'error': str(error)}
    time.sleep(3)
last.update({'schema': 'creg-r34-observer3-equivocation-canary-v1', 'slot': 'o3', 'role': 'observer',
             'source_revision': rev, 'pre': pre, 'preserved_container': preserved, 'data_backup': backup,
             'equivocation_detection': 'enabled via docker -e; node.env unchanged',
             'slashing': False, 'rpc_configuration_changed': False, 'validator_rollout': False,
             'captured_at_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())})
pathlib.Path(evidence).parent.mkdir(parents=True, exist_ok=True)
pathlib.Path(evidence).write_text(json.dumps(last, indent=2) + '\n')
print(json.dumps({'passed': last.get('passed', False),
                  'failed_checks': [k for k, v in last.get('checks', {}).items() if v is not True],
                  'error': last.get('error'), 'informational': last.get('informational'),
                  'evidence': evidence}))
if not last.get('passed'):
    raise SystemExit('r34 canary failed readiness; both containers and the data backup are kept')
PY
[[ $(sudo docker inspect "$preserved" --format '{{.State.Status}}') == exited ]]
echo "r34 equivocation canary passed on observer-3. Old container: $preserved. Data backup: $backup. Evidence: $evidence"
