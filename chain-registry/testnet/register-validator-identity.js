#!/usr/bin/env node
// ==============================================================================
// Chain Registry (CREG) — Dual-Signed Validator Identity Registration
// ==============================================================================

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

let ethers;
const candidatePaths = [
    './scripts/evm-tool/node_modules/ethers',
    '../scripts/evm-tool/node_modules/ethers',
    '../../scripts/evm-tool/node_modules/ethers',
    'ethers'
];
for (const p of candidatePaths) {
    try {
        ethers = require(p).ethers || require(p);
        if (ethers) break;
    } catch (_) {}
}

const DEFAULT_VAL_KEY_PATH = path.join(process.env.USERPROFILE || '', '.creg-validator', 'validator.key');

function parseArgs() {
    const args = process.argv.slice(2);
    const parsed = {
        evmKey: null,
        valKey: null,
        nodeId: 'ext-val-primary',
        alias: 'External Validator Primary',
        chainId: 'creg-testnet-1',
        endpoints: ['http://localhost:8080', 'https://api.testnet.cregnet.dev']
    };

    for (let i = 0; i < args.length; i++) {
        const arg = args[i];
        if (arg === '--key' || arg === '-k') {
            parsed.evmKey = args[++i];
        } else if (arg === '--validator-key' || arg === '-v') {
            parsed.valKey = args[++i];
        } else if (arg === '--node-id' || arg === '-n') {
            parsed.nodeId = args[++i];
        } else if (arg === '--alias') {
            parsed.alias = args[++i];
        } else if (arg === '--help' || arg === '-h') {
            printHelp();
            process.exit(0);
        }
    }
    return parsed;
}

function printHelp() {
    console.log(`
\x1b[36mChain Registry (CREG) — Dual-Signed Validator Registration\x1b[0m

\x1b[1mCOMMANDS:\x1b[0m
  --key <EVM_PRIVATE_KEY>        Ethereum private key for your staked wallet
  --validator-key <HEX_OR_PATH>  Ed25519 validator private key (default: ~/.creg-validator/validator.key)
  --node-id <NODE_ID>            Node ID (default: ext-val-primary)
  --alias <NAME>                 Display name (default: External Validator Primary)

\x1b[1mEXAMPLE:\x1b[0m
  node testnet/register-validator-identity.js --key 0xYOUR_SEPOLIA_PRIVATE_KEY
`);
}

function getEd25519Key(valKeyInput) {
    let hex = valKeyInput;
    if (!hex && fs.existsSync(DEFAULT_VAL_KEY_PATH)) {
        hex = fs.readFileSync(DEFAULT_VAL_KEY_PATH, 'utf8').trim();
    }
    if (!hex) {
        throw new Error('Validator key not found. Provide --validator-key <HEX> or create ~/.creg-validator/validator.key');
    }
    hex = hex.replace(/^0x/i, '').trim();
    const keyBytes = Buffer.from(hex, 'hex');
    const privKey = crypto.createPrivateKey({
        key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), keyBytes]),
        format: 'der',
        type: 'pkcs8'
    });
    const pubKey = crypto.createPublicKey(privKey);
    const pubkeyHex = pubKey.export({ format: 'der', type: 'spki' }).slice(12).toString('hex');
    return { privKey, pubkeyHex };
}

async function main() {
    const opts = parseArgs();

    console.log('\x1b[36m============================================================\x1b[0m');
    console.log('\x1b[36m   CREG Dual-Signed Validator Identity Registration        \x1b[0m');
    console.log('\x1b[36m============================================================\x1b[0m');

    if (!opts.evmKey) {
        console.log('\x1b[33m[Notice] Please provide your Sepolia private key using --key 0x...\x1b[0m');
        printHelp();
        process.exit(1);
    }

    const { privKey: edPrivKey, pubkeyHex: edPubkey } = getEd25519Key(opts.valKey);
    const wallet = new ethers.Wallet(opts.evmKey);
    const evmAddress = wallet.address.toLowerCase();
    const nonce = Date.now().toString();

    console.log(`  EVM Address    : \x1b[32m${evmAddress}\x1b[0m`);
    console.log(`  Node ID        : \x1b[32m${opts.nodeId}\x1b[0m`);
    console.log(`  Ed25519 Pubkey : \x1b[32m${edPubkey}\x1b[0m`);
    console.log(`  Alias          : \x1b[33m${opts.alias}\x1b[0m`);
    console.log(`  Nonce          : \x1b[90m${nonce}\x1b[0m\n`);

    const message = [
        'creg-validator-identity-v1',
        `chain_id:${opts.chainId}`,
        `evm_address:${evmAddress}`,
        `node_id:${opts.nodeId}`,
        `ed25519_pubkey:${edPubkey}`,
        `nonce:${nonce}`
    ].join('\n');

    console.log('Generating dual cryptographic signatures...');
    // 1. Sign with EVM key (EIP-191 personal_sign)
    const evmSignature = await wallet.signMessage(message);

    // 2. Sign with Ed25519 node key
    const edSignature = crypto.sign(null, Buffer.from(message, 'utf8'), edPrivKey).toString('hex');

    const payload = {
        evm_address: evmAddress,
        node_id: opts.nodeId,
        ed25519_pubkey: edPubkey,
        alias: opts.alias,
        nonce: nonce,
        evm_signature: evmSignature,
        ed25519_signature: edSignature
    };

    console.log('\x1b[32m  ✓ Dual signatures generated successfully!\x1b[0m\n');

    for (const base of opts.endpoints) {
        try {
            console.log(`Submitting registration to ${base}/v1/validators/register ...`);
            const res = await fetch(`${base}/v1/validators/register`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(payload)
            });
            const data = await res.json().catch(() => null);
            if (res.ok) {
                console.log(`\x1b[32m  ✓ Successfully registered on ${base}!\x1b[0m`);
                if (data) console.log('    Response:', JSON.stringify(data, null, 2));
            } else {
                console.warn(`\x1b[33m  ⚠ ${base} returned status ${res.status}: ${data?.error || res.statusText}\x1b[0m`);
            }
        } catch (e) {
            console.warn(`\x1b[33m  ⚠ Failed to reach ${base}: ${e.message}\x1b[0m`);
        }
    }

    console.log('\n\x1b[32m============================================================\x1b[0m');
    console.log('\x1b[32m   Validator Identity Registration Complete!                \x1b[0m');
    console.log('\x1b[32m============================================================\x1b[0m\n');
}

main().catch(err => {
    console.error('\x1b[31m[Error]\x1b[0m', err.message);
    process.exit(1);
});
