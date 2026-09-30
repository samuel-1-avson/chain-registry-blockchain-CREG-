// Snapshot a real upstream package and real OSV answer. Does not install,
// execute package code, publish, sign, or alter the fleet scanner profile.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

async function request(url, options = {}) {
  const response = await fetch(url, { ...options, signal: AbortSignal.timeout(30000) });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > 2 * 1024 * 1024) throw new Error('Unexpectedly large package/evidence response');
  return bytes;
}
async function main() {
  const directory = process.argv[2];
  if (!directory) throw new Error('Provide a new evidence directory');
  if (fs.existsSync(directory)) throw new Error('Evidence directory already exists; preserve it');
  const name = process.argv[3] || 'escape-string-regexp';
  const version = process.argv[4] || '5.0.0';
  if (!/^[a-z0-9][a-z0-9._-]*$/i.test(name) || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error('Package name and exact semantic version are required');
  }
  const canonical = `npm:${name}@${version}`;
  const registryUrl = `https://registry.npmjs.org/${encodeURIComponent(name)}/${encodeURIComponent(version)}`;
  const metadataBytes = await request(registryUrl);
  const metadata = JSON.parse(metadataBytes);
  if (metadata.name !== name || metadata.version !== version) throw new Error('Unexpected registry identity');
  const archiveUrl = new URL(metadata.dist.tarball);
  if (archiveUrl.protocol !== 'https:' || archiveUrl.hostname !== 'registry.npmjs.org') throw new Error('Unexpected registry archive origin');
  const archive = await request(archiveUrl.toString());
  const expected = metadata.dist.integrity;
  if (!expected || !expected.startsWith('sha512-')) throw new Error('Required registry SHA-512 integrity missing');
  const integrity = 'sha512-' + crypto.createHash('sha512').update(archive).digest('base64');
  if (integrity !== expected) throw new Error('Registry archive integrity mismatch');
  const query = { package: { ecosystem: 'npm', name: metadata.name }, version: metadata.version };
  const osvBytes = await request('https://api.osv.dev/v1/query', {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(query),
  });
  const osv = JSON.parse(osvBytes);
  if (osv.vulns !== undefined && !Array.isArray(osv.vulns)) throw new Error('Invalid OSV response shape');
  const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
  const report = {
    captured_at_utc: new Date().toISOString(), status: 'prepared_for_review_not_published', canonical,
    registry_url: registryUrl, archive_url: archiveUrl.toString(), archive_bytes: archive.length,
    archive_sha256: hash(archive), registry_sha512_integrity: integrity,
    metadata_sha256: hash(metadataBytes), osv_source: 'https://api.osv.dev/v1/query',
    osv_query: query, osv_response_sha256: hash(osvBytes), osv_advisory_count: (osv.vulns || []).length,
    declared_license: metadata.license, declared_scripts: metadata.scripts || {},
    declared_dependencies: metadata.dependencies || {},
    prerequisites: ['Confirm canonical ID absent from R7', 'Review exact archive contents',
      'Add genuine OSV response to a new pinned scanner profile', 'Roll out and verify the profile on all eight nodes',
      'Use actual publisher authorization and stake', 'Complete reviewed node rollout before the V4 crash capture'],
    fleet_changed: false, package_executed_or_published: false,
  };
  fs.mkdirSync(directory, { recursive: false });
  for (const [filename, bytes] of Object.entries({ 'registry.json': metadataBytes, 'package.tgz': archive,
    'osv-response.json': osvBytes, 'osv-query.json': JSON.stringify(query, null, 2) + '\n',
    'review.json': JSON.stringify(report, null, 2) + '\n' })) fs.writeFileSync(path.join(directory, filename), bytes);
  console.log(JSON.stringify(report, null, 2));
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
