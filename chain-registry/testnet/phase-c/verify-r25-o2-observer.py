#!/usr/bin/env python3
"""Read-only acceptance capture for the real R25 O2 observer canary."""

import hashlib
import json
import pathlib
import subprocess
import time
import urllib.request

ROOT = pathlib.Path('/var/lib/creg/phase-c-r7')
IMAGE = 'sha256:2a553f1cb8b2ea6c63bf99649c57e0291698e3ac01da5de80863760afcc07a70'
BINARY = '24925bf0df62ed9fafe4565f9969ba83c0beefbbd6725abbd8814f99ea04cf07'
PROFILE = 'f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'
BLOCK = '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()


def get(path):
    with urllib.request.urlopen('http://127.0.0.1:8080' + path, timeout=8) as response:
        return response.read().decode()


def sha256(path):
    digest = hashlib.sha256()
    with pathlib.Path(path).open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


container = json.loads(docker('inspect', 'creg-phase-c-r7-o2'))[0]
preserved = json.loads(docker('inspect', 'creg-phase-c-r7-o2-r22-before-r25'))[0]
stats = json.loads(get('/v1/public/chain/stats'))
health = json.loads(get('/v1/public/health'))
pbft = json.loads(get('/v1/consensus/pbft'))
metrics = get('/metrics')
mounts = {(m['Source'], m['Destination'], m['RW']) for m in container['Mounts']}
binary_sha = docker('exec', 'creg-phase-c-r7-o2', 'sha256sum', '/app/creg-node').split()[0]
checks = {
    'image_and_binary': container['Image'] == IMAGE and binary_sha == BINARY,
    'running_healthy': container['State']['Running'] and container['State']['Health']['Status'] == 'healthy',
    'prior_preserved_stopped': preserved['State']['Status'] == 'exited',
    'same_identity': sha256(ROOT / 'data/p2p/identity.key') == '602c79f9b4fd0d1b63c9a59b5a44fa91c1a3f03e470a78f5203b80065745e177',
    'same_environment': sha256(ROOT / 'node.env') == '7aac4ca0a264cd1290be192a018e80a5257e2cf35db0718d2a9aa604e8fe26d7',
    'same_mounts': (str(ROOT / 'data'), '/data', True) in mounts and
        (str(ROOT / 'scanner-r11-osv-kleur'), '/scanner', False) in mounts,
    'signed_r7_h14': stats['chain_id'] == 'creg-phase-c-testnet-2026-09-26-r7'
        and stats['genesis_hash'] == '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
        and stats['tip_height'] == stats['finalized_height'] == 14
        and stats['tip_hash'] == BLOCK and stats['spec_signature_verified'] is True,
    'four_real_validators': stats['validator_count'] == stats['active_validators'] == 4,
    'observer_role': health['node_id'] == 'observer-2' and health['is_validator'] is False,
    'peers_recovered': stats['peer_count'] > 0,
    'sandbox_isolated': health['sandbox']['isolated'] is True and health['sandbox']['degraded'] is False,
    'scanner_ready': 'profile_digest="' + PROFILE + '"' in metrics
        and 'creg_scanner_profile_ready 1' in metrics
        and 'creg_scanner_osv_ready 1' in metrics,
    'queues_clear': 'creg_pending_block_txs 0' in metrics
        and 'creg_durable_finalized_work_depth 0' in metrics,
    'pbft_clear': not pbft.get('rounds'),
}
report = {
    'schema': 'creg-real-r25-o2-observer-recovery-v1',
    'captured_at_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
    'node_id': 'observer-2',
    'image_id': container['Image'],
    'node_binary_sha256': binary_sha,
    'height': stats['tip_height'],
    'block_hash': stats['tip_hash'],
    'peer_count': stats['peer_count'],
    'checks': checks,
    'passed': all(checks.values()),
    'initial_startup_check_failed': True,
    'public_chain_cutover': False,
}
path = ROOT / 'evidence/r25-o2-observer-recovery.json'
path.parent.mkdir(parents=True, exist_ok=True)
with path.open('x', encoding='utf-8') as destination:
    json.dump(report, destination, indent=2)
    destination.write('\n')
print(json.dumps(report))
if not report['passed']:
    raise SystemExit('O2 recovery acceptance failed')
