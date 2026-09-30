#!/usr/bin/env python3
"""Fail-closed, read-only R7 V4 pre/post image-rollout checks on its real VM."""

import hashlib
import json
import pathlib
import re
import subprocess
import sys
import time
import urllib.request

stage = sys.argv[1]
if stage not in ('pre', 'post'):
    raise SystemExit('stage must be pre or post')

root = pathlib.Path('/var/lib/creg/phase-c-r7')
old_image = 'sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9'
new_image = 'sha256:704cd7dfb89a048c146a3de502416c31a6f77c0e3f493a98f4fe3098c732046c'
new_binary = '24925bf0df62ed9fafe4565f9969ba83c0beefbbd6725abbd8814f99ea04cf07'
profile = 'f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'
block = '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()


def sha256(path):
    digest = hashlib.sha256()
    with pathlib.Path(path).open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def get(path):
    with urllib.request.urlopen('http://127.0.0.1:8080' + path, timeout=8) as response:
        return response.read().decode()


def metric(metrics, name):
    match = re.search(r'^' + re.escape(name) + r'\s+([-+]?[0-9.]+)$', metrics, re.M)
    return float(match.group(1)) if match else None


container = json.loads(docker('inspect', 'creg-phase-c-r7-v4'))[0]
stats = json.loads(get('/v1/public/chain/stats'))
health = json.loads(get('/v1/public/health'))
pbft = json.loads(get('/v1/consensus/pbft'))
metrics = get('/metrics')
mounts = {(m['Source'], m['Destination'], m['RW']) for m in container['Mounts']}
environment = (root / 'node.env').read_text(encoding='utf-8')
unresolved = [item for item in pbft.get('rounds', [])
              if item.get('phase') != 'finalised' and
              (item.get('phase') != 'failed' or item.get('prepare_count', 0) >= item.get('quorum', 3))]
binary_sha = docker('exec', 'creg-phase-c-r7-v4', 'sha256sum', '/app/creg-node').split()[0]
identity_sha = sha256(root / 'data/p2p/identity.key')
environment_sha = sha256(root / 'node.env')
checks = {
    'expected_image': container['Image'] == (old_image if stage == 'pre' else new_image),
    'running_healthy': container['State']['Running'] and container['State']['Health']['Status'] == 'healthy',
    'same_identity': identity_sha == 'c287fdb093e8f8a2458ad36206ee50c3918cbb41b4e5c5795f5f77d834f8f575',
    'same_environment': environment_sha == 'c37b9d363172638aa18271df57ead06caff2f82abc2ae6e64e573bfdb88be8ae',
    'validator_role_and_quorum': 'CREG_NODE_ID=validator-4\n' in environment
        and 'CREG_NODE_ROLE=validator\n' in environment
        and 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false\n' in environment
        and not any(line.startswith('CREG_PHASE_C_COMMIT_CAPTURE_') for line in environment.splitlines()),
    'journal_present': (root / 'data/pbft-safety.rocksdb').is_dir(),
    'same_mounts': (str(root / 'data'), '/data', True) in mounts
        and (str(root / 'scanner-r11-osv-kleur'), '/scanner', False) in mounts
        and ('/var/run/docker.sock', '/var/run/docker.sock', True) in mounts
        and container['HostConfig']['Privileged'] is True,
    'signed_r7_h14': stats['chain_id'] == 'creg-phase-c-testnet-2026-09-26-r7'
        and stats['genesis_hash'] == '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
        and stats['tip_height'] == stats['finalized_height'] == 14
        and stats['tip_hash'] == block and stats['spec_signature_verified'] is True,
    'four_real_validators': stats['active_validators'] == stats['validator_count'] == 4,
    'live_peers': stats['peer_count'] > 0,
    'validator_healthy': health['node_id'] == 'validator-4' and health['is_validator'] is True
        and health['sandbox']['engine'] == 'nsjail'
        and health['sandbox']['isolated'] is True
        and health['sandbox']['degraded'] is False,
    'scanner_ready': 'profile_digest="' + profile + '"' in metrics
        and metric(metrics, 'creg_scanner_profile_ready') == 1
        and metric(metrics, 'creg_scanner_osv_ready') == 1,
    'queues_clear': metric(metrics, 'creg_pending_block_txs') == 0
        and metric(metrics, 'creg_durable_finalized_work_depth') == 0
        and metric(metrics, 'creg_forced_inclusion_pending') == 0,
    'six_terminal_records_preserved': metric(metrics, 'creg_durable_finalized_work_dead_letter_depth') == 6,
    'pbft_clear': not unresolved,
    'new_binary': stage == 'pre' or binary_sha == new_binary,
}
report = {
    'schema': 'creg-real-r25-v4-validator-rollout-check-v1',
    'stage': stage,
    'captured_at_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
    'node_id': 'validator-4',
    'image_id': container['Image'],
    'node_binary_sha256': binary_sha,
    'identity_sha256': identity_sha,
    'environment_sha256': environment_sha,
    'height': stats['tip_height'],
    'block_hash': stats['tip_hash'],
    'peer_count': stats['peer_count'],
    'dead_letter_depth': metric(metrics, 'creg_durable_finalized_work_dead_letter_depth'),
    'checks': checks,
    'passed': all(checks.values()),
    'public_chain_cutover': False,
}
print(json.dumps(report))
if not report['passed']:
    raise SystemExit('V4 ' + stage + ' rollout verification failed')
