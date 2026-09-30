#!/usr/bin/env node
// Print only the public key corresponding to an operator-supplied Ed25519 seed.
const crypto = require('node:crypto');
const seed = (process.env.CREG_PHASE_C_SEED || '').trim().replace(/^0x/i, '');
if (!/^[0-9a-fA-F]{64}$/.test(seed)) {
  console.error('CREG_PHASE_C_SEED must be a 32-byte hex seed');
  process.exit(2);
}
const pkcs8 = Buffer.concat([
  Buffer.from('302e020100300506032b657004220420', 'hex'),
  Buffer.from(seed, 'hex'),
]);
const privateKey = crypto.createPrivateKey({ key: pkcs8, format: 'der', type: 'pkcs8' });
const publicKey = crypto.createPublicKey(privateKey)
  .export({ format: 'der', type: 'spki' })
  .subarray(-32);
console.log(publicKey.toString('hex'));
