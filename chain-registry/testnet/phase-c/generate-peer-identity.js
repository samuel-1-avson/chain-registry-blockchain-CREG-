#!/usr/bin/env node
// Create a persistent libp2p Ed25519 identity for a real Phase C bootnode.
// The private protobuf file must live on that node's private data disk.
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const output = process.argv[2];
if (!output || process.argv.length !== 3) {
  console.error('Usage: node generate-peer-identity.js <private identity.key path>');
  process.exit(2);
}

const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function base58(bytes) {
  let value = BigInt(`0x${bytes.toString('hex')}`);
  let result = '';
  while (value > 0n) {
    result = alphabet[Number(value % 58n)] + result;
    value /= 58n;
  }
  for (const byte of bytes) {
    if (byte !== 0) break;
    result = `1${result}`;
  }
  return result;
}

const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519');
const seed = privateKey.export({ format: 'der', type: 'pkcs8' }).subarray(-32);
const pubkey = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
// libp2p PrivateKey protobuf: Type=Ed25519, Data=seed || public key.
const privateEnvelope = Buffer.concat([
  Buffer.from([0x08, 0x01, 0x12, 0x40]), seed, pubkey,
]);
const publicEnvelope = Buffer.concat([
  Buffer.from([0x08, 0x01, 0x12, 0x20]), pubkey,
]);
const peerId = base58(Buffer.concat([Buffer.from([0x00, 0x24]), publicEnvelope]));

fs.mkdirSync(path.dirname(output), { recursive: true, mode: 0o700 });
fs.writeFileSync(output, privateEnvelope, { flag: 'wx', mode: 0o600 });
try { fs.chmodSync(output, 0o600); } catch (error) {
  if (process.platform !== 'win32') throw error;
}
console.log(peerId);
