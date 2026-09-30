#!/usr/bin/env node
// Validate a preserved libp2p Ed25519 identity and print only its public peer ID.
const crypto = require('node:crypto');
const fs = require('node:fs');

if (process.argv.length !== 3) {
  console.error('Usage: node read-peer-identity.js <private identity.key path>');
  process.exit(2);
}

function base58(bytes) {
  const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  let value = BigInt('0x' + bytes.toString('hex'));
  let result = '';
  while (value > 0n) {
    result = alphabet[Number(value % 58n)] + result;
    value /= 58n;
  }
  for (const byte of bytes) {
    if (byte !== 0) break;
    result = '1' + result;
  }
  return result;
}

try {
  const envelope = fs.readFileSync(process.argv[2]);
  if (envelope.length !== 68 || !envelope.subarray(0, 4).equals(Buffer.from([8, 1, 18, 64]))) {
    throw new Error('Unsupported identity envelope');
  }
  const privateKey = crypto.createPrivateKey({
    key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), envelope.subarray(4, 36)]),
    format: 'der',
    type: 'pkcs8',
  });
  const publicKey = crypto.createPublicKey(privateKey).export({ format: 'der', type: 'spki' }).subarray(-32);
  if (!crypto.timingSafeEqual(publicKey, envelope.subarray(36, 68))) {
    throw new Error('Identity public key does not match its seed');
  }
  const publicEnvelope = Buffer.concat([Buffer.from([8, 1, 18, 32]), publicKey]);
  console.log(base58(Buffer.concat([Buffer.from([0, 36]), publicEnvelope])));
} catch {
  console.error('Could not validate the preserved P2P identity; private data was suppressed.');
  process.exit(1);
}
