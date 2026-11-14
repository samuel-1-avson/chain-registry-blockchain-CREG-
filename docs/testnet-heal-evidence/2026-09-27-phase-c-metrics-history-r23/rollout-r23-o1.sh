#!/usr/bin/env bash
set -Eeuo pipefail

slot=o1
role=observer
container=creg-phase-c-r7-o1
base=/var/lib/creg/phase-c-r7
data="$base/data"
scanner="$base/scanner-r11-osv-kleur"
env_file="$base/node.env"
image_ref=us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node:phase-c-secure-r23-metrics-history
registry_ref=us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:ef21b958b5e3e4f09e1c87290d3d9a50f23b5350d207e5af9982dd1cf5413708
expected_image_id=sha256:ef21b958b5e3e4f09e1c87290d3d9a50f23b5350d207e5af9982dd1cf5413708
old_image_id=sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3
node_sha=b6cab56f893b13cb0340834d7f69a9c23eb80f12d139186fb1056e5ce2e12972
cli_sha=b987ba41fe68efde6b33cad2f6a7e8d4784402902094b6b78e0742a19d0705b7
chain_id=creg-phase-c-testnet-2026-09-26-r7
block_genesis_hash=0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4
network_genesis_hash=1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234
scanner_epoch=osv-2026-09-27-phase-c-kleur-r11
scanner_profile=f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c
tip_height=14
tip_hash=2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa
api=http://127.0.0.1:18080
run_id=$(date -u +%Y%m%dT%H%M%SZ)
preserved="$container-r22-preserved-r23-metrics-history-$run_id"
report="$base/evidence/r23-o1-metrics-history-$run_id.json"
mutation_started=false
finish() {
  status=$?
  trap - EXIT
  if (( status != 0 )) && [[ "$mutation_started" == true ]]; then
    echo "R23 O1 rollout stopped with status $status. The preserved container and current replacement were retained for inspection; no automatic rollback was attempted." >&2
  fi
  exit "$status"
}
trap finish EXIT

exec 9>"/tmp/creg-r23-o1-rollout.lock"
flock -n 9
for path in "$env_file" "$data/p2p/identity.key" "$data/chain.rocksdb" "$scanner/osv_snapshot.json" "$scanner/manifest.json" "$scanner/rules/supply_chain.yar"; do
  sudo test -e "$path"
done
! sudo docker container inspect "$preserved" >/dev/null 2>&1
[[ $(sudo docker inspect "$container" --format '{{.State.Status}} {{.State.Running}}') == 'running true' ]]
[[ $(sudo docker inspect "$container" --format '{{.Image}}') == "$old_image_id" ]]
sudo grep -Fxq 'CREG_NODE_ID=observer-1' "$env_file"
sudo grep -Fxq 'CREG_NODE_ROLE=observer' "$env_file"
sudo grep -Fxq 'CREG_IS_VALIDATOR=false' "$env_file"
sudo grep -Fxq "CREG_CHAIN_ID=$chain_id" "$env_file"
sudo grep -Fxq 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$env_file"
sudo grep -Fxq "CREG_OSV_SNAPSHOT_EPOCH=$scanner_epoch" "$env_file"
sudo grep -Fxq "CREG_SCANNER_PROFILE_DIGEST=$scanner_profile" "$env_file"
if sudo grep -q '^CREG_VALIDATOR_KEY=' "$env_file"; then
  echo 'Observer environment unexpectedly contains a validator key.' >&2
  exit 1
fi
mounts=$(sudo docker inspect "$container" --format '{{range .Mounts}}{{println .Source "|" .Destination "|" .RW}}{{end}}')
grep -Fq "$data | /data | true" <<<"$mounts"
grep -Fq "$scanner | /scanner | false" <<<"$mounts"
sudo docker inspect "$container" | sudo python3 -c 'import json,subprocess,sys; c=json.load(sys.stdin)[0]; nets=c["NetworkSettings"]["Networks"]; ports=c["HostConfig"]["PortBindings"]; assert {"bridge","monitoring_default"}.issubset(nets); assert ports["8080/tcp"][0]["HostPort"]=="18080"; assert ports["9000/tcp"][0]["HostPort"]=="9001"; assert c["HostConfig"]["RestartPolicy"]["Name"]=="unless-stopped"; image=json.loads(subprocess.check_output(["sudo","docker","image","inspect",c["Image"],"--format","{{json .Config.Env}}"],text=True)) or []; env={x.split("=",1)[0] for x in c["Config"]["Env"]}; fileenv={x.split("=",1)[0] for x in open("/var/lib/creg/phase-c-r7/node.env") if "=" in x}; imageenv={x.split("=",1)[0] for x in image}; assert env==fileenv | imageenv'
[[ $(sudo docker image inspect "$image_ref" --format '{{.Id}}') == "$expected_image_id" ]]
repo_digests=$(sudo docker image inspect "$image_ref" --format '{{join .RepoDigests "\n"}}')
grep -Fxq "$registry_ref" <<<"$repo_digests"
[[ $(sudo docker run --rm --entrypoint /app/creg-node "$image_ref" --version) == 'creg-node 0.1.0' ]]
[[ $(sudo docker run --rm --entrypoint /app/creg "$image_ref" --version) == 'creg 0.1.0' ]]
sudo docker run --rm --entrypoint /bin/bash "$image_ref" -lc "printf '%s  %s\\n' '$node_sha' /app/creg-node '$cli_sha' /app/creg | sha256sum -c - >/dev/null"
identity_before=$(sudo sha256sum "$data/p2p/identity.key" | cut -d' ' -f1)
env_sha_before=$(sudo sha256sum "$env_file" | cut -d' ' -f1)

sudo python3 - "$api" "$chain_id" "$block_genesis_hash" "$network_genesis_hash" "$tip_height" "$tip_hash" "$scanner_profile" <<'PY'
import json, re, sys, urllib.error, urllib.request
api, chain, block_genesis, network_genesis, height, tip, profile = sys.argv[1:]
def get(path):
    with urllib.request.urlopen(api + path, timeout=12) as response:
        return response.read().decode()
stats = json.loads(get('/v1/public/chain/stats'))
health = json.loads(get('/v1/public/health'))
metrics = get('/metrics')
rounds = json.loads(get('/v1/consensus/pbft')).get('rounds', [])
assert stats.get('chain_id') == chain and stats.get('spec_signature_verified') is True
assert stats.get('genesis_hash', '').removeprefix('0x') == block_genesis
assert stats.get('tip_height') == int(height) and stats.get('finalized_height') == int(height) and stats.get('tip_hash') == tip
assert stats.get('validator_count') == 4 and stats.get('active_validators') == 4 and stats.get('peer_count', 0) > 0
assert health.get('is_validator') is False
assert f'profile_digest="{profile}"' in metrics
assert re.search(r'^creg_scanner_profile_ready(?:\{[^}]*\})?\s+1(?:\.0)?$', metrics, re.M)
for name in ('creg_pending_block_txs', 'creg_durable_finalized_work_depth', 'creg_durable_finalized_work_ready_depth', 'creg_forced_inclusion_pending'):
    match = re.search(r'^' + re.escape(name) + r'(?:\{[^}]*\})?\s+([0-9.]+)$', metrics, re.M)
    assert match and float(match.group(1)) == 0, name
assert all(r.get('phase') in ('finalised', 'failed') for r in rounds)
try:
    get('/v1/public/metrics/history?range=5m')
except urllib.error.HTTPError as error:
    assert error.code == 404
else:
    raise AssertionError('Expected the old R22 observer to lack the metrics history route')
PY

mutation_started=true
sudo docker network disconnect -f monitoring_default "$container"
sudo docker stop --time 20 "$container" >/dev/null
sudo docker rename "$container" "$preserved"
sudo docker run -d --name "$container" --restart unless-stopped --env-file "$env_file" \
  --network bridge \
  -p 18080:8080 -p 9001:9000 \
  --mount "type=bind,source=$data,target=/data" \
  --mount "type=bind,source=$scanner,target=/scanner,readonly" \
  "$image_ref" >/dev/null
sudo docker network connect --alias "$container" monitoring_default "$container"

sudo python3 - "$api" "$container" "$preserved" "$image_ref" "$registry_ref" "$expected_image_id" "$old_image_id" "$node_sha" "$cli_sha" "$chain_id" "$block_genesis_hash" "$network_genesis_hash" "$tip_height" "$tip_hash" "$scanner_epoch" "$scanner_profile" "$identity_before" "$env_sha_before" "$report" <<'PY'
import datetime, hashlib, json, pathlib, re, subprocess, sys, time, urllib.error, urllib.parse, urllib.request
(api, container, preserved, image_ref, registry_ref, expected_image_id, old_image_id,
 node_sha, cli_sha, chain, block_genesis, network_genesis, expected_height, expected_tip,
 scanner_epoch, scanner_profile, identity_before, env_sha_before, report_path) = sys.argv[1:]
expected_height = int(expected_height)
def get(path):
    with urllib.request.urlopen(api + path, timeout=12) as response:
        return response.read().decode()
def metric_value(metrics, name):
    match = re.search(r'^' + re.escape(name) + r'(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)$', metrics, re.M)
    return float(match.group(1)) if match else None
last_error = None
for _ in range(180):
    try:
        health = json.loads(get('/v1/public/health'))
        stats = json.loads(get('/v1/public/chain/stats'))
        if (health.get('is_validator') is False and stats.get('chain_id') == chain and
                stats.get('spec_signature_verified') is True and
                stats.get('genesis_hash', '').removeprefix('0x') == block_genesis and
                stats.get('tip_height') == expected_height and stats.get('tip_hash') == expected_tip and
                stats.get('validator_count') == 4 and stats.get('active_validators') == 4 and
                stats.get('peer_count', 0) > 0):
            break
        last_error = {'peer_count': stats.get('peer_count'), 'active_validators': stats.get('active_validators'), 'tip_height': stats.get('tip_height')}
    except Exception as error:
        last_error = str(error)
    time.sleep(2)
else:
    raise RuntimeError('O1 identity, API and peer readiness did not converge: ' + str(last_error))
metrics = get('/metrics')
rounds = json.loads(get('/v1/consensus/pbft')).get('rounds', [])
history = json.loads(get('/v1/public/metrics/history?range=5m'))
assert health.get('is_validator') is False
assert stats.get('chain_id') == chain and stats.get('spec_signature_verified') is True
assert stats.get('genesis_hash', '').removeprefix('0x') == block_genesis
assert stats.get('tip_height') == expected_height and stats.get('finalized_height') == expected_height and stats.get('tip_hash') == expected_tip
assert stats.get('validator_count') == 4 and stats.get('active_validators') == 4 and stats.get('peer_count', 0) > 0
assert history.get('status') == 'ready' and history.get('collector') == 'prometheus'
assert history.get('chain_id') == chain and history.get('network_genesis_hash') == network_genesis
samples = history.get('samples', [])
assert len(samples) == history.get('sample_count') and len(samples) >= 3
assert history.get('queried_series', 0) >= 1
height_samples = [sample for sample in samples if 'height' in sample]
assert height_samples
latest_height = int(float(height_samples[-1]['height']))
assert latest_height == expected_height
now_ms = int(time.time() * 1000)
latest_timestamp_ms = int(height_samples[-1]['timestamp_ms'])
assert 0 <= now_ms - latest_timestamp_ms <= 120_000
assert f'profile_digest="{scanner_profile}"' in metrics
for name in ('creg_scanner_profile_ready', 'creg_scanner_osv_ready', 'creg_scanner_yara_ready', 'creg_scanner_threat_intel_ready'):
    assert metric_value(metrics, name) == 1, name
assert metric_value(metrics, 'creg_scanner_threat_intel_entries') == 100
queues = {name: metric_value(metrics, name) for name in ('creg_pending_block_txs', 'creg_durable_finalized_work_depth', 'creg_durable_finalized_work_ready_depth', 'creg_durable_finalized_work_dead_letter_depth', 'creg_forced_inclusion_pending')}
assert all(queues[name] == 0 for name in ('creg_pending_block_txs', 'creg_durable_finalized_work_depth', 'creg_durable_finalized_work_ready_depth', 'creg_forced_inclusion_pending'))
assert all(r.get('phase') in ('finalised', 'failed') for r in rounds)
query = f'creg_chain_tip_height{{chain_id="{chain}",network_genesis_hash="{network_genesis}",metrics_source="explorer-primary"}}'
raw_prometheus = subprocess.check_output(['docker', 'exec', container, 'curl', '-fsS', '--get', '--data-urlencode', 'query=' + query, 'http://creg-gcp-prometheus:9090/api/v1/query'], text=True, timeout=12)
prom_payload = json.loads(raw_prometheus)
assert prom_payload.get('status') == 'success'
prom_series = prom_payload.get('data', {}).get('result', [])
assert len(prom_series) == 1
labels = prom_series[0].get('metric', {})
assert labels.get('chain_id') == chain and labels.get('network_genesis_hash') == network_genesis and labels.get('metrics_source') == 'explorer-primary'
prom_height = int(float(prom_series[0]['value'][1]))
assert prom_height == latest_height
actual_image = subprocess.check_output(['docker', 'inspect', container, '--format', '{{.Image}}'], text=True).strip()
assert actual_image == expected_image_id
repo_digests = subprocess.check_output(['docker', 'image', 'inspect', actual_image, '--format', '{{join .RepoDigests "\n"}}'], text=True).splitlines()
assert registry_ref in repo_digests or ('creg-node@' + registry_ref.rsplit('@', 1)[-1]) in repo_digests
actual_node_sha = subprocess.check_output(['docker', 'exec', container, 'sha256sum', '/app/creg-node'], text=True).split()[0]
actual_cli_sha = subprocess.check_output(['docker', 'exec', container, 'sha256sum', '/app/creg'], text=True).split()[0]
assert actual_node_sha == node_sha and actual_cli_sha == cli_sha
identity_after = hashlib.sha256(pathlib.Path('/var/lib/creg/phase-c-r7/data/p2p/identity.key').read_bytes()).hexdigest()
env_sha_after = hashlib.sha256(pathlib.Path('/var/lib/creg/phase-c-r7/node.env').read_bytes()).hexdigest()
assert identity_after == identity_before and env_sha_after == env_sha_before
old_status = subprocess.check_output(['docker', 'inspect', preserved, '--format', '{{.State.Status}}'], text=True).strip()
assert old_status == 'exited'
network_json = subprocess.check_output(['docker', 'inspect', container, '--format', '{{json .NetworkSettings.Networks}}'], text=True)
networks = sorted(json.loads(network_json).keys())
assert {'bridge', 'monitoring_default'}.issubset(networks)
mounts_json = subprocess.check_output(['docker', 'inspect', container, '--format', '{{json .Mounts}}'], text=True)
mounts = json.loads(mounts_json)
assert any(m['Source'] == '/var/lib/creg/phase-c-r7/data' and m['Destination'] == '/data' and m['RW'] for m in mounts)
assert any(m['Source'] == '/var/lib/creg/phase-c-r7/scanner-r11-osv-kleur' and m['Destination'] == '/scanner' and not m['RW'] for m in mounts)
record = {
    'schema': 'creg-phase-c-r23-o1-metrics-history-rollout-v1',
    'captured_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'target': {'slot': 'o1', 'role': 'observer', 'vm': 'creg-testnet-vm', 'project': 'gen-lang-client-0022105784', 'zone': 'us-central1-b'},
    'chain': {'chain_id': chain, 'block_genesis_hash': block_genesis, 'network_genesis_hash': network_genesis, 'tip_height': stats['tip_height'], 'tip_hash': stats['tip_hash'], 'finalized_height': stats['finalized_height'], 'signature_verified': stats['spec_signature_verified']},
    'image': {'ref': image_ref, 'registry_digest': registry_ref, 'image_id': actual_image, 'previous_image_id': old_image_id, 'node_sha256': actual_node_sha, 'cli_sha256': actual_cli_sha},
    'preserved_container': preserved,
    'data_identity_sha256_before_after': identity_after,
    'environment_sha256_before_after': env_sha_after,
    'networks': networks,
    'scanner': {'epoch': scanner_epoch, 'profile_digest': scanner_profile, 'ready': metric_value(metrics, 'creg_scanner_profile_ready') == 1},
    'queues': queues,
    'pbft_clear': all(r.get('phase') in ('finalised', 'failed') for r in rounds),
    'metrics_history': {'http': 200, 'collector': history['collector'], 'status': history['status'], 'range': history['range'], 'sample_count': len(samples), 'queried_series': history['queried_series'], 'sample_fields': sorted({key for sample in samples for key in sample if key != 'timestamp_ms'}), 'latest_sample_timestamp_ms': latest_timestamp_ms, 'latest_height': latest_height},
    'direct_prometheus_cross_check': {'series_count': len(prom_series), 'chain_id': labels['chain_id'], 'network_genesis_hash': labels['network_genesis_hash'], 'metrics_source': labels['metrics_source'], 'latest_height': prom_height},
    'public_cutover': False,
    'chain_reset': False,
}
pathlib.Path(report_path).parent.mkdir(parents=True, exist_ok=True)
pathlib.Path(report_path).write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record, separators=(',', ':')))
PY

