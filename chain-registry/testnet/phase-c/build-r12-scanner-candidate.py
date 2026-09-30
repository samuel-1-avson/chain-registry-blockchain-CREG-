#!/usr/bin/env python3
"""Extend the real R11 scanner snapshot with the reviewed OSV response for R12."""

import datetime
import hashlib
import json
import pathlib
import shutil
import sys


def sha(data):
    return hashlib.sha256(data).hexdigest()


def profile_digest(bundle, epoch):
    files = sorted(bundle.joinpath('rules').glob('*.yar'), key=lambda path: path.name)
    if not files:
        raise ValueError('YARA rule set is empty')
    hasher = hashlib.sha256()
    hasher.update(b'creg-yara-rule-bundle-v1\0')
    for path in files:
        for item in (path.name.encode(), path.read_bytes()):
            hasher.update(len(item).to_bytes(8, 'big'))
            hasher.update(item)
    yara = hasher.hexdigest()
    threat = sha(bundle.joinpath('known_malicious_hashes.json').read_bytes())
    osv = sha(bundle.joinpath('osv_snapshot.json').read_bytes())
    profile = (f'creg-scanner-profile-v1|version=creg-deterministic-scanner-v1.1.0'
               f'|yara={yara}|threat={threat}|osv_enabled=true|osv_epoch={epoch}'
               f'|osv={osv}|osv_block_critical=true|osv_live_fallback=false')
    return sha(profile.encode()), yara, threat, osv


repo = pathlib.Path(__file__).resolve().parents[3]
evidence = repo / 'docs/testnet-heal-evidence'
old = evidence / '2026-09-27-phase-c-implementation-start/cli-write/scanner-r11-bundle'
package = evidence / '2026-09-27-production-pipeline-cutover/next-real-package'
new = evidence / '2026-09-27-production-pipeline-cutover/scanner-r12-bundle'
epoch = 'osv-2026-09-27-phase-c-escape-string-regexp-r12'
canonical = 'npm:escape-string-regexp@5.0.0'

old_manifest = json.loads(old.joinpath('manifest.json').read_text(encoding='utf-8'))
old_snapshot = json.loads(old.joinpath('osv_snapshot.json').read_text(encoding='utf-8'))
old_digest, old_yara, old_threat, old_osv = profile_digest(old, old_snapshot['epoch'])
assert old_digest == old_manifest['scanner_profile_digest'] == 'f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'
assert old_yara == old_manifest['yara_bundle_sha256']
assert old_threat == old_manifest['threat_database_sha256']
assert old_osv == old_manifest['osv_snapshot_sha256']

review = json.loads(package.joinpath('review.json').read_text(encoding='utf-8'))
query = json.loads(package.joinpath('osv-query.json').read_text(encoding='utf-8'))
response_bytes = package.joinpath('osv-response.json').read_bytes()
response = json.loads(response_bytes)
assert review['canonical'] == canonical
assert query == review['osv_query']
assert sha(response_bytes) == review['osv_response_sha256']
assert response == {} or response == {'vulns': []}
assert review['osv_advisory_count'] == 0
assert canonical not in old_snapshot['entries']
assert not new.exists(), 'R12 candidate already exists; never overwrite evidence'

new.mkdir(parents=True)
new.joinpath('rules').mkdir()
shutil.copy2(old / 'known_malicious_hashes.json', new / 'known_malicious_hashes.json')
for path in old.joinpath('rules').iterdir():
    if path.is_file():
        shutil.copy2(path, new / 'rules' / path.name)

now = datetime.datetime.now(datetime.timezone.utc).isoformat().replace('+00:00', 'Z')
snapshot = dict(old_snapshot)
snapshot['epoch'] = epoch
snapshot['built_at'] = now
snapshot['entries'] = dict(old_snapshot['entries'])
snapshot['entries'][canonical] = []
new.joinpath('osv_snapshot.json').write_text(
    json.dumps(snapshot, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')

digest, yara, threat, osv = profile_digest(new, epoch)
manifest = dict(old_manifest)
manifest['generated_at_utc'] = now
manifest['osv_epoch'] = epoch
manifest['osv_snapshot_sha256'] = osv
manifest['scanner_profile_digest'] = digest
manifest['osv_coverage'] = list(old_manifest['osv_coverage']) + [
    {'key': canonical, 'vulnerability_count': 0}
]
new.joinpath('manifest.json').write_text(
    json.dumps(manifest, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')

report = {
    'schema': 'creg-real-scanner-r12-candidate-v1',
    'captured_at_utc': now,
    'status': 'prepared_not_deployed',
    'canonical': canonical,
    'new_profile_digest': digest,
    'old_profile_digest': old_digest,
    'osv_epoch': epoch,
    'osv_response_sha256': sha(response_bytes),
    'osv_advisory_count': 0,
    'osv_snapshot_sha256': osv,
    'threat_database_sha256': threat,
    'yara_bundle_sha256': yara,
    'coverage_count': len(snapshot['entries']),
    'prior_real_threat_feed_reused': True,
    'node_restarts_or_publication': False,
}
new.joinpath('review.json').write_text(
    json.dumps(report, indent=2) + '\n', encoding='utf-8')
print(json.dumps(report))
