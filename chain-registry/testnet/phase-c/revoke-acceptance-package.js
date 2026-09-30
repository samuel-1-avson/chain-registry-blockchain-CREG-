#!/usr/bin/env node
// Submit a real, publisher-signed revocation after the package is finalized.
const crypto = require('node:crypto');
const fs = require('node:fs');

const [keyPath, requestPath, baseUrl] = process.argv.slice(2);
if (!keyPath || !requestPath || !baseUrl) {
  console.error('Usage: node revoke-acceptance-package.js <publisher-ed25519.key> <signed-publish.json> <candidate-api-url>');
  process.exit(2);
}

async function main() {
  const request = JSON.parse(fs.readFileSync(requestPath, 'utf8'));
  const canonical = `${request.id.ecosystem}:${request.id.name}@${request.id.version}`;
  const keyHex = fs.readFileSync(keyPath, 'utf8').trim();
  if (!/^[0-9a-fA-F]{64}$/.test(keyHex)) throw new Error('Invalid publisher Ed25519 key file');
  const pkcs8 = Buffer.concat([
    Buffer.from('302e020100300506032b657004220420', 'hex'),
    Buffer.from(keyHex, 'hex'),
  ]);
  const privateKey = crypto.createPrivateKey({ key: pkcs8, format: 'der', type: 'pkcs8' });
  const pubkey = crypto.createPublicKey(privateKey)
    .export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
  if (pubkey !== request.publisher_pubkey) throw new Error('Publisher key does not match the signed publish request');

  const endpoint = `${baseUrl.replace(/\/$/, '')}/v1/packages/${encodeURIComponent(canonical)}`;
  const existing = await fetch(endpoint);
  if (!existing.ok) throw new Error(`Package is not readable yet: HTTP ${existing.status}`);
  const record = await existing.json();
  const status = record.status ?? record.package?.status;
  if (status !== 'Verified' && status !== 'verified') {
    throw new Error(`Package is not finalized as verified: ${JSON.stringify(status)}`);
  }

  const reason = 'Phase C live revocation after shared publication';
  const statsResponse = await fetch(`${baseUrl.replace(/\/$/, '')}/v1/public/chain/stats`);
  if (!statsResponse.ok) throw new Error('Chain identity is unavailable');
  const stats = await statsResponse.json();
  if (!stats.spec_signature_verified || !stats.chain_id || !/^[a-f0-9]{64}$/.test(stats.genesis_hash)) {
    throw new Error('Authenticated chain identity is required for revocation');
  }
  const signature = crypto.sign(
    null,
    Buffer.from(`creg-revoke-v1:${stats.chain_id}:${stats.genesis_hash}:${canonical}:revoke:${reason}`, 'utf8'),
    privateKey,
  ).toString('hex');
  const revocation = { reason, revoker_pubkey: pubkey, signature, signature_version: 1,
    chain_id: stats.chain_id, genesis_hash: stats.genesis_hash };
  fs.writeFileSync(`${requestPath}.revocation.json`, JSON.stringify(revocation, null, 2) + '\n');
  const response = await fetch(`${endpoint}/revoke`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(revocation),
  });
  const body = await response.text();
  console.log(`HTTP ${response.status} ${body}`);
  if (!response.ok) process.exitCode = 1;
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
