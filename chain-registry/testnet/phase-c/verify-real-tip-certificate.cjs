// Read-only guard for replacing a real node at an independently accepted tip.
const fs = require('node:fs');
const crypto = require('node:crypto');
const [api, specPath, heightRaw, expectedHash] = process.argv.slice(2);
const height = Number(heightRaw);
const digest = value => crypto.createHash('sha256').update(value).digest('hex');
function canonical(value) {
  if (Array.isArray(value)) return '[' + value.map(canonical).join(',') + ']';
  if (value && typeof value === 'object') return '{' + Object.keys(value).sort().map(key =>
    JSON.stringify(key) + ':' + canonical(value[key])).join(',') + '}';
  if (typeof value === 'number' && !Number.isSafeInteger(value)) throw new Error('Unsupported consensus numeric encoding');
  return JSON.stringify(value);
}
(async () => {
  const bytes = fs.readFileSync(specPath);
  if (digest(bytes) !== '331897f5a3f89fe902e4a7d814a9245ef50b384e22cc3d8dc946b4b45ed39e9f') throw new Error('Authenticated R7 spec pin changed');
  const spec = JSON.parse(bytes);
  const response = await fetch(`${api}/v1/public/blocks/${height}`, { signal: AbortSignal.timeout(8000) });
  if (!response.ok) throw new Error(`Block read failed: ${response.status}`);
  const block = await response.json();
  const header = { ...block.header }; delete header.view_change_certificate;
  if (header.view_number === 0) delete header.view_number;
  if (block.header.height !== height || !block.finalized || block.hash !== expectedHash ||
      digest(JSON.stringify(header)) !== expectedHash) throw new Error('Accepted canonical tip hash differs');
  let leaves = block.transactions.length ? block.transactions.map(tx => digest(canonical(tx))) : [digest('empty')];
  while (leaves.length > 1) {
    if (leaves.length % 2) leaves.push(leaves.at(-1));
    const next = []; for (let i = 0; i < leaves.length; i += 2) next.push(digest(leaves[i] + leaves[i + 1]));
    leaves = next;
  }
  if (leaves[0] !== header.merkle_root) throw new Error('Merkle root differs');
  const roster = new Map(spec.validator_set.validators.map(v => [v.id, v.pubkey]));
  const signers = new Set();
  for (const sig of block.pbft_signatures) {
    if (signers.has(sig.validator_id) || roster.get(sig.validator_id) !== sig.pubkey) throw new Error('Wrong or duplicate COMMIT signer');
    const key = crypto.createPublicKey({ format: 'der', type: 'spki', key: Buffer.concat([
      Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(sig.pubkey, 'hex')]) });
    if (!crypto.verify(null, Buffer.from(`creg-pbft-v1:commit:${expectedHash}`), key, Buffer.from(sig.signature, 'hex'))) throw new Error('Invalid COMMIT');
    signers.add(sig.validator_id);
  }
  if (roster.size !== 4 || signers.size < 3) throw new Error('Real four-validator quorum missing');
  console.log(JSON.stringify({ passed: true, height, hash: expectedHash, commit_signers: [...signers] }));
})().catch(error => { console.error(error.message); process.exitCode = 1; });
