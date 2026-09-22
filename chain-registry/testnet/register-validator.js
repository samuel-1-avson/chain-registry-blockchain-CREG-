#!/usr/bin/env node
// ==============================================================================
// Chain Registry (CREG) — Validator Identity Registration Helper (testnet)
// ==============================================================================
// POSTs a dual-signed identity proof to a node's /v1/validator/register so a
// staked operator's node joins the consensus set (chain-authoritative sync
// admits L1-active addresses only once their node identity is registered).
//
// Two signing flows for the EVM key:
//   A. Automatic: --evm-key-file <path-to-hex-key> (or --evm-key <hex>).
//      Key never leaves this machine; delete the file after use.
//   B. Manual (recommended for wallet-held keys): run once with --print-message,
//      personal_sign the printed message in MetaMask/wallet, then re-run with
//      the SAME --nonce plus --evm-address and --evm-signature.
//
// The Ed25519 key always comes from --ed-key-file (raw 32-byte hex, e.g. the
// validator.key produced by `creg keygen validator` with an empty passphrase).
//
// Usage:
//   node register-validator.js --node-url http://127.0.0.1:8080 \
//     --node-id validator-1 --ed-key-file ~/.creg-validator/validator.key \
//     --evm-key-file ~/.creg-validator/evm.key [--alias "my validator"]
//   node register-validator.js --selftest   # no network, verifies crypto wiring
// ==============================================================================

const fs = require('fs');
const crypto = require('crypto');

const MESSAGE_HEADER = 'creg-validator-identity-v1';

function arg(name, def = undefined) {
  const i = process.argv.indexOf(`--${name}`);
  if (i === -1) return def;
  return process.argv[i + 1];
}
function hasFlag(name) {
  return process.argv.includes(`--${name}`);
}
function readHexFile(path, label, bytes) {
  const hex = fs.readFileSync(path, 'utf8').trim().replace(/^0x/i, '').toLowerCase();
  if (!/^[0-9a-f]+$/.test(hex) || hex.length !== bytes * 2) {
    throw new Error(`${label} must be ${bytes} bytes of hex (file: ${path})`);
  }
  return hex;
}

// Ed25519 via Node's built-in crypto (no deps). Seed -> PKCS8 DER -> KeyObject.
const PKCS8_PREFIX = Buffer.from('302e020100300506032b657004220420', 'hex');
function edPrivateKey(seedHex) {
  return crypto.createPrivateKey({
    key: Buffer.concat([PKCS8_PREFIX, Buffer.from(seedHex, 'hex')]),
    format: 'der',
    type: 'pkcs8',
  });
}
function edPubkeyHex(seedHex) {
  const spki = crypto.createPublicKey(edPrivateKey(seedHex)).export({ format: 'der', type: 'spki' });
  return spki.subarray(-32).toString('hex'); // SPKI BIT STRING tail = raw 32B pubkey
}
function edSign(seedHex, message) {
  return crypto.sign(null, Buffer.from(message, 'utf8'), edPrivateKey(seedHex)).toString('hex');
}
function edVerify(pubkeyHex, message, sigHex) {
  const spki = Buffer.concat([
    Buffer.from('302a300506032b6570032100', 'hex'),
    Buffer.from(pubkeyHex, 'hex'),
  ]);
  const pub = crypto.createPublicKey({ key: spki, format: 'der', type: 'spki' });
  return crypto.verify(null, Buffer.from(message, 'utf8'), pub, Buffer.from(sigHex, 'hex'));
}

function buildMessage({ chainId, evmAddress, nodeId, edPubkey, nonce }) {
  return (
    `${MESSAGE_HEADER}\n` +
    `chain_id:${chainId}\n` +
    `evm_address:${evmAddress.toLowerCase()}\n` +
    `node_id:${nodeId}\n` +
    `ed25519_pubkey:${edPubkey.toLowerCase().replace(/^0x/, '')}\n` +
    `nonce:${nonce}`
  );
}

function loadEthers() {
  const candidates = [
    '../scripts/evm-tool/node_modules/ethers',
    './scripts/evm-tool/node_modules/ethers',
    '../../scripts/evm-tool/node_modules/ethers',
    './node_modules/ethers',
    'ethers',
  ];
  for (const p of candidates) {
    try {
      const mod = require(p);
      const ethers = mod.ethers || mod;
      if (ethers && ethers.Wallet) return ethers;
    } catch (_) {}
  }
  throw new Error('ethers not found (needed for --evm-key auto-signing); use --evm-signature manual flow instead');
}

async function selftest() {
  // 1. Ed25519 sign/verify round-trip (catches DER-wrapping bugs).
  const seed = crypto.randomBytes(32).toString('hex');
  const pub = edPubkeyHex(seed);
  if (pub.length !== 64) throw new Error('pubkey length wrong');
  const sig = edSign(seed, 'selftest-message');
  if (sig.length !== 128) throw new Error('signature length wrong');
  if (!edVerify(pub, 'selftest-message', sig)) throw new Error('round-trip verify failed');
  if (edVerify(pub, 'tampered', sig)) throw new Error('verify accepted tampered message');
  // 2. Message format must match the node's validator_identity_registration_message exactly.
  const msg = buildMessage({
    chainId: 'creg-testnet-1',
    evmAddress: '0x5F424B4EC88073FD461EB194833681A31aDFA311',
    nodeId: 'validator-1',
    edPubkey: 'AB6D12BE97E73D2090ADA1042B274C9020F395A798E2CE82CC1DBD711D0643AB',
    nonce: 'abc123',
  });
  const expected =
    'creg-validator-identity-v1\n' +
    'chain_id:creg-testnet-1\n' +
    'evm_address:0x5f424b4ec88073fd461eb194833681a31adfa311\n' +
    'node_id:validator-1\n' +
    'ed25519_pubkey:ab6d12be97e73d2090ada1042b274c9020f395a798e2ce82cc1dbd711d0643ab\n' +
    'nonce:abc123';
  if (msg !== expected) throw new Error(`message format mismatch:\n${msg}`);
  console.log('selftest: Ed25519 round-trip OK, message format OK');
}

async function main() {
  if (hasFlag('selftest')) {
    await selftest();
    return;
  }
  const nodeUrl = arg('node-url');
  const nodeId = arg('node-id');
  const edKeyFile = arg('ed-key-file');
  const chainId = arg('chain-id', 'creg-testnet-1');
  const alias = arg('alias', undefined);
  if (!nodeUrl || !nodeId || !edKeyFile) {
    throw new Error('required: --node-url, --node-id, --ed-key-file');
  }
  const edSeed = readHexFile(edKeyFile, 'Ed25519 key', 32);
  const edPubkey = edPubkeyHex(edSeed);
  console.log(`Ed25519 pubkey: ${edPubkey}  (compare with your keygen output)`);

  let evmAddress, evmSignature;
  const manualSig = arg('evm-signature', undefined);
  if (manualSig) {
    evmAddress = arg('evm-address', undefined);
    if (!evmAddress || !/^0x[0-9a-fA-F]{40}$/.test(evmAddress)) {
      throw new Error('--evm-signature requires --evm-address 0x… (40 hex)');
    }
    evmSignature = manualSig.trim().toLowerCase().replace(/^0x/, '');
    if (!/^[0-9a-f]{130}$/.test(evmSignature)) throw new Error('--evm-signature must be 65 bytes hex');
  } else {
    const ethers = loadEthers();
    const evmKey = arg('evm-key', undefined) || (arg('evm-key-file', undefined)
      ? readHexFile(arg('evm-key-file'), 'EVM key', 32)
      : undefined);
    if (!evmKey || !/^[0-9a-f]{64}$/i.test(evmKey)) {
      throw new Error('need --evm-key-file/--evm-key, or manual --evm-signature + --evm-address');
    }
    var wallet = new ethers.Wallet('0x' + evmKey.toLowerCase().replace(/^0x/, ''));
    evmAddress = wallet.address;
  }

  const nonce = arg('nonce', crypto.randomBytes(16).toString('hex'));
  const message = buildMessage({ chainId, evmAddress, nodeId, edPubkey, nonce });
  if (hasFlag('print-message')) {
    console.log('--- personal_sign this exact message, then re-run with --nonce, --evm-address, --evm-signature ---');
    console.log(`nonce: ${nonce}`);
    console.log(message);
    return;
  }
  if (!manualSig) {
    evmSignature = (await wallet.signMessage(message)).replace(/^0x/, '').toLowerCase();
  }
  const edSignature = edSign(edSeed, message);

  const body = {
    evm_address: evmAddress.toLowerCase(),
    node_id: nodeId,
    ed25519_pubkey: edPubkey,
    nonce,
    evm_signature: '0x' + evmSignature,
    ed25519_signature: edSignature,
    ...(alias ? { alias } : {}),
  };
  const res = await fetch(`${nodeUrl.replace(/\/$/, '')}/v1/validator/register`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const text = await res.text();
  console.log(`HTTP ${res.status}`);
  console.log(text.slice(0, 2000));
  if (!res.ok) process.exit(1);
}

main().catch((e) => {
  console.error(`Error: ${e.message}`);
  process.exit(1);
});
