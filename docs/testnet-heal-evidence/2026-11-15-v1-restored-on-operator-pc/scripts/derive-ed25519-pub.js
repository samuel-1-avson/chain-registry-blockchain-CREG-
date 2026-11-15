// Reads an Ed25519 private key (hex) on stdin and prints ONLY its public key.
const crypto = require('crypto');
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (d) => { input += d; });
process.stdin.on('end', () => {
  let s = input.trim();
  if (s.startsWith('0x')) s = s.slice(2);
  let format = 'plain-hex';
  if (!/^[0-9a-fA-F]{64}([0-9a-fA-F]{64})?$/.test(s)) {
    const runs = s.match(/[0-9a-fA-F]{64,128}/g) || [];
    if (runs.length !== 1) {
      console.log(JSON.stringify({ ok: false, length: s.length, hex_runs: runs.length }));
      return;
    }
    s = runs[0].length >= 128 ? runs[0].slice(0, 128) : runs[0].slice(0, 64);
    format = 'embedded-hex';
  }
  const seed = Buffer.from(s.slice(0, 64), 'hex');
  const der = Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), seed]);
  const pub = crypto.createPublicKey(crypto.createPrivateKey({ key: der, format: 'der', type: 'pkcs8' }));
  const spki = pub.export({ format: 'der', type: 'spki' });
  const pubkey = spki.subarray(spki.length - 32).toString('hex');
  const embedded = s.length === 128 ? s.slice(64).toLowerCase() : null;
  console.log(JSON.stringify({ ok: true, format, bytes: s.length / 2, pubkey, embedded_pub_matches: embedded ? embedded === pubkey : null }));
});
