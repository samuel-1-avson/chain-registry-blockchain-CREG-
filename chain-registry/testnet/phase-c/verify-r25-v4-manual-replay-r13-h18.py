#!/usr/bin/env python3
"""Read-only evidence for V4's real h18 COMMIT replay after a manual Docker start."""

import datetime as dt
import json
import pathlib
import re
import subprocess
import urllib.parse
import urllib.request

BASE = pathlib.Path('/var/lib/creg/phase-c-r7')
CAPTURE = BASE / 'evidence/w3-v4-durable-commit-capture-r25-r13-h18.json'
CONTAINER = 'creg-phase-c-r7-v4'
HASH = 'ec64471b8f9c58eb2d9b958db765a98871cb444e032143d313d65b3ab80cd76c'
PARENT = '7c3364858873b00dcb8eef4875c2f1eaa85ade32648850177bd12dcb5f7627d7'
PROFILE = '8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e'
ANSI = re.compile(r'\x1b\[[0-?]*[ -/]*[@-~]')


def api(path):
    with urllib.request.urlopen('http://127.0.0.1:8080' + path, timeout=10) as response:
        return response.read().decode()


def timestamp(line):
    return dt.datetime.fromisoformat(line.split(' ', 1)[0].replace('Z', '+00:00'))


capture = json.loads(CAPTURE.read_text())
info = json.loads(subprocess.check_output(['docker', 'inspect', CONTAINER], text=True))[0]
logs = subprocess.run(['docker', 'logs', '--timestamps', CONTAINER], capture_output=True, text=True, check=True)
lines = [ANSI.sub('', line) for line in (logs.stdout + logs.stderr).splitlines()]
markers = {}
for name in ('pbft_local_commit_durable', 'phase_c_durable_commit_capture_paused', 'pbft_local_commit_recovered'):
    candidates = [line for line in lines if name in line and f'block_hash={HASH}' in line
                  and 'height=18' in line and f'parent_hash={PARENT}' in line
                  and 'creg-phase-c-testnet-2026-09-26-r7' in line]
    markers[name] = candidates[-1] if candidates else None

stats = json.loads(api('/v1/public/chain/stats'))
health = json.loads(api('/v1/public/health'))
pbft = json.loads(api('/v1/consensus/pbft'))
package = json.loads(api('/v1/public/packages/' + urllib.parse.quote('npm:has-flag@5.0.1', safe='')))
metrics = api('/metrics')
kill = capture.get('kill') or {}
kill_time = dt.datetime.fromisoformat(kill['completed_at_utc']) if kill.get('completed_at_utc') else None
recovery_time = timestamp(markers['pbft_local_commit_recovered']) if markers['pbft_local_commit_recovered'] else None
checks = {
    'exact_durable_and_pause_markers': bool(markers['pbft_local_commit_durable'] and markers['phase_c_durable_commit_capture_paused']),
    'exact_sigkill_completed': kill.get('exit_code') == 0 and kill.get('signal') == 'SIGKILL',
    'same_container_running_after_manual_start': info['Id'] == capture['container_id'] and info['State']['Running'],
    'same_block_recovered_after_kill': bool(recovery_time and kill_time and recovery_time > kill_time),
    'recovery_before_p2p_sync_start': bool(markers['pbft_local_commit_recovered'] and 'before P2P and sync services start' in markers['pbft_local_commit_recovered']),
    'canonical_h18_and_revocation': stats.get('tip_height') == stats.get('finalized_height') == 18
        and stats.get('tip_hash') == HASH and package.get('status', '').lower() == 'revoked'
        and package.get('block_phase') == 'committed',
    'validator_healthy': health.get('status') == 'ok' and health.get('node_id') == 'validator-4',
    'scanner_r13_ready': f'profile_digest="{PROFILE}"' in metrics and 'creg_scanner_profile_ready 1' in metrics,
    'queues_and_pbft_clear': 'creg_pending_block_txs 0' in metrics and 'creg_durable_finalized_work_depth 0' in metrics
        and 'creg_forced_inclusion_pending 0' in metrics and pbft.get('active_round_count') == 0 and not pbft.get('rounds'),
    'journal_preserved': (BASE / 'data/pbft-safety.rocksdb').is_dir(),
}
report = {
    'schema': 'creg-r25-v4-manual-start-durable-replay-r13-h18-v1',
    'captured_at_utc': dt.datetime.now(dt.timezone.utc).isoformat(),
    'container_id': info['Id'], 'image_id': info['Image'],
    'restart_count': info['RestartCount'], 'manual_start_required': True,
    'height': 18, 'block_hash': HASH, 'parent_hash': PARENT,
    'kill': kill, 'markers': markers, 'checks': checks,
    'passed': all(checks.values()),
}
print(json.dumps(report, indent=2))
if not report['passed']:
    raise SystemExit(1)
