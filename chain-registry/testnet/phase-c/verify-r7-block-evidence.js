// Verify public block hashes and distinct Ed25519 COMMIT signatures from real nodes.
const fs = require('node:fs');
const crypto = require('node:crypto');
const path = require('node:path');

const [specPath, directory, heightsArgument = '1,2', expectedPackage = 'npm:is-number@7.0.0',
  expectedStatus = 'revoked', expectedArtifactHash = '', expectedProfile = ''] = process.argv.slice(2);
if (!specPath || !directory) throw new Error('Usage: node verify-r7-block-evidence.js <spec> <evidence-directory> [heights]');
const spec = JSON.parse(fs.readFileSync(specPath, 'utf8').replace(/^\uFEFF/, ''));
const heights = heightsArgument.split(',').map(Number);
if (heights.some(h => !Number.isSafeInteger(h) || h < 1)) throw new Error('Invalid block heights');
const roster = new Map(spec.validator_set.validators.map(v => [v.id, v.pubkey]));
if (roster.size !== 4 || spec.chain_id !== 'creg-phase-c-testnet-2026-09-26-r7') throw new Error('Unexpected signed roster');
const slots = ['v1', 'v2', 'v3', 'v4', 'o1', 'o2', 'o3', 'o4'];
const sha256 = value => crypto.createHash('sha256').update(value).digest('hex');
// This captured acceptance payload uses ASCII keys and safe integer numbers.
// Reject unsupported scalar cases rather than claim general serde_json parity.
function sortedJson(value) {
  if (Array.isArray(value)) return '[' + value.map(sortedJson).join(',') + ']';
  if (value && typeof value === 'object') {
    const keys = Object.keys(value).sort();
    if (keys.some(key => /[^\x00-\x7f]/.test(key))) throw new Error('Non-ASCII keys require the Rust consensus encoder');
    return '{' + keys.map(key => JSON.stringify(key) + ':' + sortedJson(value[key])).join(',') + '}';
  }
  if (typeof value === 'number' && !Number.isSafeInteger(value)) throw new Error('Non-integer scalar requires the Rust consensus encoder');
  return JSON.stringify(value);
}
function merkleRoot(transactions) {
  if (!transactions.length) return sha256('empty');
  let hashes = transactions.map(transaction => sha256(sortedJson(transaction)));
  while (hashes.length > 1) {
    if (hashes.length % 2) hashes.push(hashes.at(-1));
    const next = [];
    for (let i = 0; i < hashes.length; i += 2) next.push(sha256(hashes[i] + hashes[i + 1]));
    hashes = next;
  }
  return hashes[0];
}
const sharedHashes = new Map();
const nodes = [];
for (const slot of slots) {
  const evidence = JSON.parse(fs.readFileSync(path.join(directory, `${slot}.json`), 'utf8').replace(/^\uFEFF/, ''));
  if (evidence.stats.chain_id !== spec.chain_id || !evidence.stats.spec_signature_verified) throw new Error(`${slot}: wrong chain context`);
  if (evidence.stats.genesis_hash.replace(/^0x/, '') !== '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4') throw new Error(`${slot}: wrong actual genesis`);
  const checks = [];
  for (const height of heights) {
    const block = evidence.blocks[String(height)];
    if (!block || block.header.height !== height || !block.finalized) throw new Error(`${slot}: missing finalized height ${height}`);
    const header = { ...block.header };
    delete header.view_change_certificate;
    if (header.view_number === 0) delete header.view_number;
    const hash = crypto.createHash('sha256').update(JSON.stringify(header)).digest('hex');
    if (hash !== block.hash) throw new Error(`${slot}: header hash mismatch at ${height}`);
    if (merkleRoot(block.transactions) !== header.merkle_root) throw new Error(`${slot}: transaction Merkle root mismatch at ${height}`);
    if (sharedHashes.has(height) && sharedHashes.get(height) !== hash) throw new Error(`Divergent block at height ${height}`);
    sharedHashes.set(height, hash);
    const signers = new Set();
    for (const signature of block.pbft_signatures) {
      if (signers.has(signature.validator_id) || roster.get(signature.validator_id) !== signature.pubkey) throw new Error(`${slot}: invalid or duplicate COMMIT signer`);
      const key = crypto.createPublicKey({ format: 'der', type: 'spki', key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(signature.pubkey, 'hex')]) });
      if (!crypto.verify(null, Buffer.from(`creg-pbft-v1:commit:${hash}`), key, Buffer.from(signature.signature, 'hex'))) throw new Error(`${slot}: invalid COMMIT signature`);
      signers.add(signature.validator_id);
    }
    if (signers.size < 3) throw new Error(`${slot}: COMMIT quorum missing`);
    const parent = evidence.blocks[String(height - 1)];
    if (parent && block.header.prev_hash !== parent.hash) throw new Error(`${slot}: parent continuity mismatch`);
    checks.push({ height, hash, commit_signers: [...signers], transaction_count: block.transactions.length });
  }
  if (heights.includes(1)) {
    const publication = evidence.blocks['1'].transactions[0];
    if (publication?.type !== 'publish_authorized_v1' || publication.record.id.name !== 'is-number' || publication.record.id.version !== '7.0.0' || publication.record.content_hash !== '7b75c1057198cf97696909a9bee176c9c5e9bcb5b03bf3ecef2f484defadd51e') throw new Error(`${slot}: expected real npm publication is missing`);
    const approvals = publication.record.validator_signatures;
    if (new Set(approvals.map(v => v.validator_id)).size < 3 || approvals.some(v => v.vote !== 'Approve' || roster.get(v.validator_id) !== v.validator_pubkey || v.ml_model_version !== 'creg-deterministic-scanner-v1.1.0+sha256:648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f')) throw new Error(`${slot}: publication lacks the expected pinned-profile approvals`);
  }
  if (heights.includes(4)) {
    const revocation = evidence.blocks['4'].transactions[0];
    if (revocation?.type !== 'revoke' || revocation.package_canonical !== 'npm:is-number@7.0.0') throw new Error(`${slot}: expected h4 revocation transaction is missing`);
  }
  if (evidence.package.canonical !== expectedPackage || evidence.package.status !== expectedStatus || evidence.package.block_phase !== 'committed') throw new Error(`${slot}: requested package state is not committed`);
  if (expectedArtifactHash && evidence.package.content_hash !== expectedArtifactHash) throw new Error(`${slot}: package artifact hash differs from the real signed tarball`);
  if (expectedStatus === 'verified') {
    const publications = heights.flatMap(height => evidence.blocks[String(height)].transactions.map(transaction => ({ height, transaction })));
    const publication = publications.find(({ transaction }) => transaction.type === 'publish_authorized_v1' &&
      `${transaction.record.id.ecosystem}:${transaction.record.id.name}@${transaction.record.id.version}` === expectedPackage);
    if (!publication || evidence.package.block_hash !== evidence.blocks[String(publication.height)].hash || publication.transaction.record.content_hash !== evidence.package.content_hash) throw new Error(`${slot}: verified package does not match its committed publication`);
    const approvals = publication.transaction.record.validator_signatures;
    if (new Set(approvals.map(vote => vote.validator_id)).size < 3 || approvals.some(vote => vote.vote !== 'Approve' || roster.get(vote.validator_id) !== vote.validator_pubkey ||
      (expectedProfile && vote.ml_model_version !== `creg-deterministic-scanner-v1.1.0+sha256:${expectedProfile}`))) throw new Error(`${slot}: publication lacks real roster and scanner-profile approval quorum`);
  }
  nodes.push({ slot, captured_at_utc: evidence.captured_at_utc, blocks: checks });
}
const result = { schema: 'creg-real-fleet-block-quorum-check-v1', checked_at_utc: new Date().toISOString(), chain_id: spec.chain_id, shared_finality: true, expected_package: expectedPackage, expected_status: expectedStatus, expected_artifact_hash: expectedArtifactHash, expected_profile_digest: expectedProfile, quorum: 3, validator_count: 4, node_count: 8, nodes };
fs.writeFileSync(path.join(directory, 'quorum-check.json'), JSON.stringify(result, null, 2) + '\n');
console.log(`All eight real nodes share finalized heights ${heights.join(',')}; header hashes, transaction Merkle roots and at least three distinct Ed25519 COMMIT signatures verified per block.`);
