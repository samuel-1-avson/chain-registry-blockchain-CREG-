#!/usr/bin/env bash
# Controlled R25 observer canary on the existing real O2 VM. No validator signs
# here. The prior container is retained and its data, env, and identity stay put.
set -euo pipefail

base=/var/lib/creg/phase-c-r7
container=creg-phase-c-r7-o2
preserved=${container}-r22-before-r25
image=us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2a553f1cb8b2ea6c63bf99649c57e0291698e3ac01da5de80863760afcc07a70
image_id=sha256:2a553f1cb8b2ea6c63bf99649c57e0291698e3ac01da5de80863760afcc07a70
prior_id=sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3
binary_sha=24925bf0df62ed9fafe4565f9969ba83c0beefbbd6725abbd8814f99ea04cf07
profile=f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c
scanner=$base/scanner-r11-osv-kleur
api=http://127.0.0.1:8080

exec 9>/tmp/creg-r25-o2-rollout.lock
flock -n 9 || { echo 'O2 rollout already in progress' >&2; exit 1; }
[[ $(sudo docker inspect -f '{{.Image}} {{.State.Status}}' "$container") == "$prior_id running" ]]
[[ $(sudo docker image inspect -f '{{.Id}}' "$image") == "$image_id" ]]
! sudo docker container inspect "$preserved" >/dev/null 2>&1
sudo test -d "$base/data/chain.rocksdb"
sudo test -f "$base/data/p2p/identity.key"
sudo test -f "$base/node.env"
sudo test -d "$scanner"
sudo grep -Fxq 'CREG_NODE_ID=observer-2' "$base/node.env"
sudo grep -Fxq 'CREG_NODE_ROLE=observer' "$base/node.env"
sudo grep -Fxq 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$base/node.env"
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$profile" "$base/node.env"
! sudo grep -q '^CREG_PHASE_C_COMMIT_CAPTURE_' "$base/node.env"
[[ $(sudo docker run --rm "$image" --version) == 'creg-node 0.1.0' ]]

sudo python3 - "$container" "$base" "$scanner" "$api" "$profile" <<'PY'
import json, subprocess, sys, urllib.request
container, base, scanner, api, profile = sys.argv[1:]
config = json.loads(subprocess.check_output(['docker', 'inspect', container]))[0]
mounts = {(m['Source'], m['Destination'], m['RW']) for m in config['Mounts']}
assert (base + '/data', '/data', True) in mounts
assert (scanner, '/scanner', False) in mounts
assert config['HostConfig']['RestartPolicy']['Name'] == 'unless-stopped'
assert config['HostConfig']['PortBindings']['8080/tcp'][0]['HostPort'] == '8080'
assert config['HostConfig']['PortBindings']['9000/tcp'][0]['HostPort'] == '9000'
def get(path):
    with urllib.request.urlopen(api + path, timeout=8) as response:
        return response.read().decode()
stats = json.loads(get('/v1/public/chain/stats'))
health = json.loads(get('/v1/public/health'))
pbft = json.loads(get('/v1/consensus/pbft'))
metrics = get('/metrics')
assert stats['chain_id'] == 'creg-phase-c-testnet-2026-09-26-r7'
assert stats['genesis_hash'] == '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
assert stats['tip_height'] == stats['finalized_height'] == 14
assert stats['tip_hash'] == '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'
assert stats['active_validators'] == stats['validator_count'] == 4
assert stats['spec_signature_verified'] is True and stats['peer_count'] > 0
assert health['node_id'] == 'observer-2' and health['is_validator'] is False
assert health['sandbox']['isolated'] is True and health['sandbox']['degraded'] is False
assert not pbft.get('rounds')
assert 'profile_digest="' + profile + '"' in metrics
for line in ('creg_scanner_profile_ready 1', 'creg_scanner_osv_ready 1',
             'creg_pending_block_txs 0', 'creg_durable_finalized_work_depth 0'):
    assert line in metrics
print(json.dumps({'stage':'preflight','node':'observer-2','height':14,
                  'hash':stats['tip_hash'],'profile':profile,'ready':True}))
PY

identity_sha=$(sudo sha256sum "$base/data/p2p/identity.key" | cut -d' ' -f1)
env_sha=$(sudo sha256sum "$base/node.env" | cut -d' ' -f1)
sudo docker stop --time 30 "$container" >/dev/null
sudo docker rename "$container" "$preserved"
sudo docker run -d --name "$container" --restart unless-stopped \
  --env-file "$base/node.env" -p 8080:8080 -p 9000:9000 \
  --mount "type=bind,source=$base/data,target=/data" \
  --mount "type=bind,source=$scanner,target=/scanner,readonly" \
  "$image" >/dev/null

deadline=$((SECONDS + 300))
until curl -fsS --max-time 5 "$api/v1/public/health" >/dev/null 2>&1; do
  if (( SECONDS >= deadline )); then
    echo 'R25 O2 did not become healthy; previous container is preserved stopped for review' >&2
    exit 1
  fi
  sleep 3
done
[[ $(sudo docker inspect -f '{{.Image}} {{.State.Status}}' "$container") == "$image_id running" ]]
[[ $(sudo docker inspect -f '{{.State.Status}}' "$preserved") == exited ]]
[[ $(sudo docker exec "$container" sha256sum /app/creg-node | cut -d' ' -f1) == "$binary_sha" ]]
[[ $(sudo sha256sum "$base/data/p2p/identity.key" | cut -d' ' -f1) == "$identity_sha" ]]
[[ $(sudo sha256sum "$base/node.env" | cut -d' ' -f1) == "$env_sha" ]]

sudo python3 - "$api" "$profile" "$image_id" "$binary_sha" "$base/evidence/r25-o2-observer-rollout.json" <<'PY'
import json, pathlib, sys, time, urllib.request
api, profile, image, binary, evidence = sys.argv[1:]
def get(path):
    with urllib.request.urlopen(api + path, timeout=8) as response:
        return response.read().decode()
stats = json.loads(get('/v1/public/chain/stats'))
health = json.loads(get('/v1/public/health'))
pbft = json.loads(get('/v1/consensus/pbft'))
metrics = get('/metrics')
checks = {
    'signed_r7_h14': stats['chain_id'] == 'creg-phase-c-testnet-2026-09-26-r7'
        and stats['genesis_hash'] == '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
        and stats['tip_height'] == stats['finalized_height'] == 14
        and stats['tip_hash'] == '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'
        and stats['spec_signature_verified'] is True,
    'observer_role': health['node_id'] == 'observer-2' and health['is_validator'] is False,
    'four_real_validators': stats['validator_count'] == stats['active_validators'] == 4,
    'live_peers': stats['peer_count'] > 0,
    'sandbox_isolated': health['sandbox']['isolated'] is True
        and health['sandbox']['degraded'] is False,
    'scanner_ready': 'profile_digest="' + profile + '"' in metrics
        and 'creg_scanner_profile_ready 1' in metrics
        and 'creg_scanner_osv_ready 1' in metrics,
    'queues_clear': 'creg_pending_block_txs 0' in metrics
        and 'creg_durable_finalized_work_depth 0' in metrics,
    'pbft_clear': not pbft.get('rounds'),
}
report = {'schema':'creg-real-r25-o2-observer-rollout-v1',
          'captured_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
          'node_id':'observer-2','image_id':image,'node_binary_sha256':binary,
          'height':stats['tip_height'],'block_hash':stats['tip_hash'],
          'scanner_profile_digest':profile,'checks':checks,'passed':all(checks.values()),
          'preserved_container':'creg-phase-c-r7-o2-r22-before-r25',
          'public_chain_cutover':False}
path = pathlib.Path(evidence)
path.parent.mkdir(parents=True, exist_ok=True)
with path.open('x', encoding='utf-8') as output:
    json.dump(report, output, indent=2)
    output.write('\n')
print(json.dumps(report))
if not report['passed']:
    raise SystemExit('R25 O2 acceptance failed; preserved state requires review')
PY
