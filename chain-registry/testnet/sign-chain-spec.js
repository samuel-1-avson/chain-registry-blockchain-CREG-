#!/usr/bin/env node
// ==============================================================================
// Chain Registry (CREG) — Chain Spec Signer (testnet ceremony tool)
// ==============================================================================
// Produces the detached Ed25519 .sig for a chain-spec.json, replicating the
// repo's canonical scheme EXACTLY (crates/common/src/chain_spec.rs):
//   message = "creg-chain-spec-v1|" + JCS(write_jcs_value/write_jcs_string)
//   signature = Ed25519_sign(message)   (hex, no 0x prefix)
//
// JCS port notes (must match the Rust implementation, not JCS-in-the-abstract):
// - Object keys sorted byte-wise (Rust String Ord). Assumes ASCII keys, true
//   for chain specs (JS UTF-16 sort differs only for non-BMP keys).
// - Strings: \" \\ \b \f \n \r \t short escapes; other chars < 0x10 become
//   \u00xx; chars 0x10-0x1F are emitted RAW (repo quirk, replicated here).
// - Numbers via minimal integer rendering; assumes integers within the
//   double-safe range (true for chain specs).
// - The signed JSON must contain exactly the ChainSpec struct fields. Any
//   drift is caught by verification (see below), which uses the repo's own code.
//
// Verification (required after every signing):
//   docker run --rm --entrypoint /app/creg -v <dir>:/spec <image> \
//     chain-spec validate /spec/chain-spec.json -s /spec/chain-spec.json.sig \
//     --require-signature
// plus a negative control: flip one byte of the spec and confirm it FAILS.
//
// Usage:
//   node sign-chain-spec.js --spec chain-spec.json --key-file ~/.creg-spec-signing.key --out chain-spec.json.sig
//   SPEC_SIGNING_KEY=<hex> node sign-chain-spec.js --spec chain-spec.json --out chain-spec.json.sig
//   node sign-chain-spec.js --selftest   # no files, verifies the port
// ==============================================================================

const fs = require('fs');
const crypto = require('crypto');

const DOMAIN = 'creg-chain-spec-v1';

function arg(name, def = undefined) {
  const i = process.argv.indexOf(`--${name}`);
  if (i === -1) return def;
  const v = process.argv[i + 1];
  if (v === undefined || v.startsWith('--')) throw new Error(`--${name} needs a value`);
  return v;
}
function hasFlag(name) {
  return process.argv.includes(`--${name}`);
}

// --- JCS port (write_jcs_value / write_jcs_string) ---
function jcsString(s) {
  let out = '"';
  for (const ch of s) {
    const cp = ch.codePointAt(0);
    if (ch === '"') out += '\\"';
    else if (ch === '\\') out += '\\\\';
    else if (cp === 0x08) out += '\\b';
    else if (cp === 0x0c) out += '\\f';
    else if (cp === 0x0a) out += '\\n';
    else if (cp === 0x0d) out += '\\r';
    else if (cp === 0x09) out += '\\t';
    else if (cp < 0x10) out += `\\u${cp.toString(16).padStart(4, '0')}`;
    else out += ch; // NOTE: 0x10-0x1F emitted raw (repo quirk), non-ASCII raw UTF-8
  }
  return out + '"';
}
function jcsNumber(n) {
  if (!Number.isInteger(n) || Math.abs(n) > Number.MAX_SAFE_INTEGER) {
    throw new Error(`JCS port supports only safe integers, got: ${n}`);
  }
  return String(n);
}
function jcsValue(v) {
  if (v === null) return 'null';
  if (v === true) return 'true';
  if (v === false) return 'false';
  if (typeof v === 'number') return jcsNumber(v);
  if (typeof v === 'string') return jcsString(v);
  if (Array.isArray(v)) return `[${v.map(jcsValue).join(',')}]`;
  if (typeof v === 'object') {
    // Byte-wise key order; ASCII-only assumption documented above.
    const keys = Object.keys(v).sort((a, b) => (Buffer.from(a).compare(Buffer.from(b))));
    return `{${keys.map((k) => `${jcsString(k)}:${jcsValue(v[k])}`).join(',')}}`;
  }
  throw new Error(`unsupported JSON value type: ${typeof v}`);
}

// --- Ed25519 (same pattern as register-validator.js) ---
const PKCS8_PREFIX = Buffer.from('302e020100300506032b657004220420', 'hex');
function edSign(seedHex, message) {
  const key = crypto.createPrivateKey({
    key: Buffer.concat([PKCS8_PREFIX, Buffer.from(seedHex, 'hex')]),
    format: 'der',
    type: 'pkcs8',
  });
  return crypto.sign(null, Buffer.from(message, 'utf8'), key).toString('hex');
}
function edPubkeyHex(seedHex) {
  const key = crypto.createPrivateKey({
    key: Buffer.concat([PKCS8_PREFIX, Buffer.from(seedHex, 'hex')]),
    format: 'der',
    type: 'pkcs8',
  });
  const spki = crypto.createPublicKey(key).export({ format: 'der', type: 'spki' });
  return spki.subarray(-32).toString('hex');
}

function canonicalMessage(spec) {
  return `${DOMAIN}|${jcsValue(spec)}`;
}

async function selftest() {
  // Byte-expected JCS fixture, hand-derived from write_jcs_value/write_jcs_string.
  const fixture = {
    z: 1,
    a: [true, false, null, 42, -7],
    m: 'q"\\é\n\x01\x11\x7f',
    nested: { b: 2, a: 1 },
    empty_obj: {},
    empty_arr: [],
  };
  // \x11 (0x11) and \x7f emitted RAW per the repo quirk; \x01 -> \u0001.
  const expected =
    '{"a":[true,false,null,42,-7],"empty_arr":[],"empty_obj":{},"m":"q\\"\\\\é\\n","nested":{"a":1,"b":2},"z":1}';
  // Build the expected string programmatically to avoid source-encoding ambiguity.
  const raw11 = String.fromCharCode(0x11);
  const raw7f = String.fromCharCode(0x7f);
  const expected2 =
    '{"a":[true,false,null,42,-7],"empty_arr":[],"empty_obj":{},"m":"q\\"\\\\é\\n\\u0001' +
    raw11 + raw7f + '","nested":{"a":1,"b":2},"z":1}';
  const got = jcsValue(fixture);
  if (got !== expected2) {
    throw new Error(`JCS mismatch:\n got: ${JSON.stringify(got)}\nwant: ${JSON.stringify(expected2)}`);
  }
  void expected;
  // Ed25519 sign/verify round-trip.
  const seed = crypto.randomBytes(32).toString('hex');
  const sig = edSign(seed, 'selftest');
  const pub = edPubkeyHex(seed);
  const spki = Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(pub, 'hex')]);
  const ok = crypto.verify(
    null, Buffer.from('selftest', 'utf8'),
    crypto.createPublicKey({ key: spki, format: 'der', type: 'spki' }),
    Buffer.from(sig, 'hex'),
  );
  if (!ok || sig.length !== 128) throw new Error('Ed25519 round-trip failed');
  // Domain prefix check.
  const msg = canonicalMessage({ b: 1 });
  if (msg !== 'creg-chain-spec-v1|{"b":1}') throw new Error(`domain/prefix wrong: ${msg}`);
  console.log('selftest: JCS vectors OK, Ed25519 round-trip OK, domain OK');
}

async function main() {
  if (hasFlag('selftest')) {
    await selftest();
    return;
  }
  const specPath = arg('spec');
  const outPath = arg('out');
  if (!specPath || !outPath) throw new Error('required: --spec <in.json> --out <out.sig>');
  let seedHex;
  const keyFile = arg('key-file', undefined);
  const keyEnv = arg('key-env', 'SPEC_SIGNING_KEY');
  if (keyFile) {
    seedHex = fs.readFileSync(keyFile, 'utf8').trim().replace(/^0x/i, '').toLowerCase();
  } else if (process.env[keyEnv]) {
    seedHex = process.env[keyEnv].trim().replace(/^0x/i, '').toLowerCase();
  } else {
    throw new Error(`no key: pass --key-file or set ${keyEnv}`);
  }
  if (!/^[0-9a-f]{64}$/.test(seedHex)) throw new Error('signing key must be 32 bytes hex');
  const spec = JSON.parse(fs.readFileSync(specPath, 'utf8'));
  const specPubkey = ((spec.signing || {}).signing_key_pubkey_hex || '').toLowerCase().replace(/^0x/, '');
  const derivedPubkey = edPubkeyHex(seedHex);
  if (specPubkey !== derivedPubkey) {
    throw new Error(
      `signing key does not match spec pubkey (spec: ${specPubkey.slice(0, 16)}…, key: ${derivedPubkey.slice(0, 16)}…); refusing to sign`,
    );
  }
  const sig = edSign(seedHex, canonicalMessage(spec));
  fs.writeFileSync(outPath, sig + '\n');
  console.log(`signed ${specPath} -> ${outPath} (${sig.slice(0, 16)}…)`);
  console.log('REQUIRED: verify with containerized `creg chain-spec validate --require-signature` + tamper negative control');
}

main().catch((e) => {
  console.error(`Error: ${e.message}`);
  process.exit(1);
});
