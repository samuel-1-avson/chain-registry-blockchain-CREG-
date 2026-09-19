#!/usr/bin/env node
// ============================================================================
// Chain Registry (CREG) — CLI Staking Utility (Sepolia L1)
// ============================================================================
// Zero external binaries required — uses built-in Node.js and ethers.js
//
// Usage:
//   node testnet/stake-validator.js --status 0x5f424b4ec88073fd461eb194833681a31adfa311
//   node testnet/stake-validator.js --key 0x... --amount 100
//   node testnet/stake-validator.js --role publisher --key 0x... --amount 1
// ============================================================================

const path = require('path');
const fs = require('fs');

// Attempt to resolve ethers from local or repo paths
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

if (!ethers) {
    console.error('\x1b[31m[Error] ethers library not found. Please run "npm install" inside scripts/evm-tool\x1b[0m');
    process.exit(1);
}

// Active Sepolia Deployments
const DEFAULTS = {
    rpcUrl: process.env.SEPOLIA_RPC_URL || process.env.CREG_ETH_RPC || 'https://explorer.testnet.cregnet.dev/rpc',
    tokenAddr: process.env.CREG_TOKEN_ADDR || process.env.TESTNET_TOKEN_ADDR || '0x167Ab975dc95b79727Ed76BD8AAb70463F6499E2',
    stakingAddr: process.env.CREG_STAKING_ADDR || process.env.TESTNET_STAKING_ADDR || '0x30604Aa6f763fbde6aDCffbd8960af3031037331',
    chainId: 11155111
};

const ERC20_ABI = [
    'function name() view returns (string)',
    'function symbol() view returns (string)',
    'function decimals() view returns (uint8)',
    'function balanceOf(address) view returns (uint256)',
    'function allowance(address owner, address spender) view returns (uint256)',
    'function approve(address spender, uint256 amount) returns (bool)'
];

const STAKING_ABI = [
    'function applyToBeValidator(uint256 amount) external',
    'function stakeAsPublisher(uint256 amount) external',
    'function validators(address) view returns (uint256 stake, uint8 state, uint256 unbondingAt, uint256 slashCount, uint256 ejectedAt, uint256 appliedAt)',
    'function publisherStakes(address) view returns (uint256)',
    'function activeValidatorCount() view returns (uint256)',
    'function minValidatorStake() view returns (uint256)',
    'function minPublisherStake() view returns (uint256)',
    'function emergencyGovernanceEnabled() view returns (bool)'
];

const STATE_NAMES = {
    0: 'None (Unregistered)',
    1: 'Pending (Awaiting consensus / governance admission)',
    2: 'Active (In consensus set)',
    3: 'Unbonding',
    4: 'Ejected / Slashed'
};

function parseArgs() {
    const args = process.argv.slice(2);
    const parsed = {
        role: 'validator',
        amount: null,
        key: null,
        keyFile: null,
        statusAddr: null,
        rpcUrl: DEFAULTS.rpcUrl,
        tokenAddr: DEFAULTS.tokenAddr,
        stakingAddr: DEFAULTS.stakingAddr
    };

    for (let i = 0; i < args.length; i++) {
        const arg = args[i];
        if (arg === '--status' || arg === '-s') {
            parsed.statusAddr = args[++i];
        } else if (arg === '--role' || arg === '-r') {
            parsed.role = args[++i].toLowerCase();
        } else if (arg === '--amount' || arg === '-a') {
            parsed.amount = parseFloat(args[++i]);
        } else if (arg === '--key' || arg === '-k') {
            parsed.key = args[++i];
        } else if (arg === '--key-file' || arg === '-f') {
            parsed.keyFile = args[++i];
        } else if (arg === '--rpc-url') {
            parsed.rpcUrl = args[++i];
        } else if (arg === '--token') {
            parsed.tokenAddr = args[++i];
        } else if (arg === '--staking') {
            parsed.stakingAddr = args[++i];
        } else if (arg === '--help' || arg === '-h') {
            printHelp();
            process.exit(0);
        }
    }
    return parsed;
}

function printHelp() {
    console.log(`
\x1b[36mChain Registry (CREG) — CLI Staking Tool (Sepolia L1)\x1b[0m

\x1b[1mCOMMANDS:\x1b[0m
  --status <ADDRESS>           Check wallet balance and staking status
  --key <HEX_KEY>              Private key to submit staking transactions
  --key-file <PATH>            Path to file containing Ethereum private key
  --amount <NUM>               Amount of tCREG to stake (default: 100 for validator, 1 for publisher)
  --role <validator|publisher> Staking role (default: validator)
  --rpc-url <URL>              Sepolia RPC URL (default: ${DEFAULTS.rpcUrl})

\x1b[1mEXAMPLES:\x1b[0m
  # Check current staking status
  node testnet/stake-validator.js --status 0x5f424b4ec88073fd461eb194833681a31adfa311

  # Stake 100 tCREG as validator
  node testnet/stake-validator.js --key 0xYOUR_SEPOLIA_PRIVATE_KEY --amount 100

  # Windows wrapper:
  .\\testnet\\stake-validator.cmd -Status 0x5f424b4ec88073fd461eb194833681a31adfa311
  .\\testnet\\stake-validator.cmd -Key 0xYOUR_SEPOLIA_PRIVATE_KEY -Amount 100
`);
}

async function main() {
    const opts = parseArgs();
    const provider = new ethers.JsonRpcProvider(opts.rpcUrl);
    const token = new ethers.Contract(opts.tokenAddr, ERC20_ABI, provider);
    const staking = new ethers.Contract(opts.stakingAddr, STAKING_ABI, provider);

    console.log('\x1b[36m============================================================\x1b[0m');
    console.log('\x1b[36m   Chain Registry (CREG) — Sepolia Staking CLI\x1b[0m');
    console.log('\x1b[36m============================================================\x1b[0m');
    console.log(`  Network RPC  : \x1b[90m${opts.rpcUrl}\x1b[0m`);
    console.log(`  Token (tCREG): \x1b[90m${opts.tokenAddr}\x1b[0m`);
    console.log(`  Staking Hub  : \x1b[90m${opts.stakingAddr}\x1b[0m`);
    console.log('');

    // Mode 1: Status query
    if (opts.statusAddr) {
        await queryStatus(opts.statusAddr, provider, token, staking);
        return;
    }

    // Mode 2: Staking transaction
    let privateKey = opts.key;
    if (!privateKey && opts.keyFile) {
        if (!fs.existsSync(opts.keyFile)) {
            console.error(`\x1b[31m[Error] Key file not found: ${opts.keyFile}\x1b[0m`);
            process.exit(1);
        }
        privateKey = fs.readFileSync(opts.keyFile, 'utf8').trim();
    }

    if (!privateKey) {
        console.log('\x1b[33m[Notice] No private key provided. Use --status <ADDR> to query or --key <KEY> to stake.\x1b[0m');
        printHelp();
        return;
    }

    // Clean and validate key
    privateKey = privateKey.trim();
    if (!privateKey.startsWith('0x')) {
        privateKey = '0x' + privateKey;
    }

    if (privateKey.length !== 66) {
        console.error('\x1b[31m[Error] Invalid private key length. Expected 64 hex characters (32 bytes) with 0x prefix.\x1b[0m');
        console.error('\x1b[33m[Hint] Make sure you are using your Ethereum/Sepolia EOA private key, not your Ed25519 validator node key.\x1b[0m');
        process.exit(1);
    }

    const wallet = new ethers.Wallet(privateKey, provider);
    const address = wallet.address;
    const amount = opts.amount || (opts.role === 'publisher' ? 1.0 : 100.0);

    console.log(`  Wallet (EOA) : \x1b[32m${address}\x1b[0m`);
    console.log(`  Role         : \x1b[33m${opts.role.toUpperCase()}\x1b[0m`);
    console.log(`  Stake Amount : \x1b[32m${amount} tCREG\x1b[0m`);
    console.log('');

    await queryStatus(address, provider, token, staking);

    const ethBal = await provider.getBalance(address);
    if (ethBal === 0n) {
        console.error('\x1b[31m[Error] This wallet has 0 Sepolia ETH for gas.\x1b[0m');
        console.error('Use the faucet first to receive testnet ETH:');
        console.error(`  creg testnet drip --faucet-url https://faucet.testnet.cregnet.dev ${address}`);
        process.exit(1);
    }

    const tCregBal = await token.balanceOf(address);
    const stakeWei = ethers.parseEther(amount.toString());

    if (tCregBal < stakeWei) {
        console.error(`\x1b[31m[Error] Insufficient tCREG balance (${ethers.formatEther(tCregBal)} tCREG available, ${amount} required).\x1b[0m`);
        console.error('Request tCREG from the faucet:');
        console.error(`  creg testnet drip --faucet-url https://faucet.testnet.cregnet.dev ${address}`);
        process.exit(1);
    }

    // Check existing validator state
    if (opts.role === 'validator') {
        try {
            const v = await staking.validators(address);
            const state = Number(v.state);
            if (state === 1) {
                console.log('\x1b[33m[Notice] This wallet is ALREADY registered as a Pending validator on L1 with ' + ethers.formatEther(v.stake) + ' tCREG staked!\x1b[0m');
                console.log('No additional staking transaction is required.');
                console.log('\n\x1b[36mNext Steps:\x1b[0m');
                console.log('  1. Ensure your node is running: creg-validator-ext-val-primary');
                console.log('  2. Node will be admitted into the active consensus set upon governance/consensus sync.');
                return;
            } else if (state === 2) {
                console.log('\x1b[32m[Notice] This wallet is ALREADY an ACTIVE validator on L1!\x1b[0m');
                return;
            }
        } catch (_) {}
    }

    const tokenSigner = token.connect(wallet);
    const stakingSigner = staking.connect(wallet);

    // Step 1: Check and approve allowance
    const allowance = await token.allowance(address, opts.stakingAddr);
    if (allowance < stakeWei) {
        console.log('\x1b[33m[Step 1/2] Approving Staking Contract to spend ' + amount + ' tCREG...\x1b[0m');
        const approveTx = await tokenSigner.approve(opts.stakingAddr, ethers.MaxUint256);
        console.log(`  Tx Hash: \x1b[90m${approveTx.hash}\x1b[0m`);
        console.log('  Waiting for Sepolia block confirmation...');
        const receipt = await approveTx.wait(1);
        console.log(`\x1b[32m  ✓ Approval confirmed in block ${receipt.blockNumber}!\x1b[0m\n`);
    } else {
        console.log('\x1b[32m[Step 1/2] Staking contract already has sufficient token allowance.\x1b[0m\n');
    }

    // Step 2: Submit stake transaction
    if (opts.role === 'validator') {
        console.log(`\x1b[33m[Step 2/2] Submitting applyToBeValidator(${amount} tCREG)...\x1b[0m`);
        const stakeTx = await stakingSigner.applyToBeValidator(stakeWei);
        console.log(`  Tx Hash: \x1b[90m${stakeTx.hash}\x1b[0m`);
        console.log('  Waiting for Sepolia block confirmation...');
        const receipt = await stakeTx.wait(1);
        console.log(`\x1b[32m  ✓ Validator application confirmed in block ${receipt.blockNumber}!\x1b[0m`);
        console.log(`  Etherscan: \x1b[36mhttps://sepolia.etherscan.io/tx/${stakeTx.hash}\x1b[0m\n`);
        console.log('\x1b[32m============================================================\x1b[0m');
        console.log('\x1b[32m   ✓ Successfully Staked as Validator on Sepolia!\x1b[0m');
        console.log('\x1b[32m============================================================\x1b[0m');
        console.log('  Status: Pending (Stake escrowed in Staking.sol)');
        console.log('  Your node is now ready for consensus admission.');
    } else {
        console.log(`\x1b[33m[Step 2/2] Submitting stakeAsPublisher(${amount} tCREG)...\x1b[0m`);
        const stakeTx = await stakingSigner.stakeAsPublisher(stakeWei);
        console.log(`  Tx Hash: \x1b[90m${stakeTx.hash}\x1b[0m`);
        console.log('  Waiting for Sepolia block confirmation...');
        const receipt = await stakeTx.wait(1);
        console.log(`\x1b[32m  ✓ Publisher stake confirmed in block ${receipt.blockNumber}!\x1b[0m`);
        console.log(`  Etherscan: \x1b[36mhttps://sepolia.etherscan.io/tx/${stakeTx.hash}\x1b[0m\n`);
    }
}

async function queryStatus(addr, provider, token, staking) {
    try {
        const [ethBal, tCregBal, allowance] = await Promise.all([
            provider.getBalance(addr),
            token.balanceOf(addr),
            token.allowance(addr, await staking.getAddress())
        ]);

        console.log(`\x1b[1mBalances for ${addr}:\x1b[0m`);
        console.log(`  Sepolia ETH (Gas) : \x1b[36m${ethers.formatEther(ethBal)} ETH\x1b[0m`);
        console.log(`  tCREG Token       : \x1b[36m${ethers.formatEther(tCregBal)} tCREG\x1b[0m`);
        console.log(`  Contract Allowance: \x1b[90m${ethers.formatEther(allowance)} tCREG\x1b[0m`);

        try {
            const v = await staking.validators(addr);
            const state = Number(v.state);
            const stateStr = STATE_NAMES[state] || `Unknown (${state})`;
            console.log(`  Validator State   : \x1b[${state === 2 ? '32' : state === 1 ? '33' : '90'}m${stateStr}\x1b[0m`);
            if (state > 0) {
                console.log(`  Staked Amount     : \x1b[32m${ethers.formatEther(v.stake)} tCREG\x1b[0m`);
                if (v.appliedAt > 0n) {
                    const date = new Date(Number(v.appliedAt) * 1000).toISOString();
                    console.log(`  Applied Timestamp : \x1b[90m${date}\x1b[0m`);
                }
            }
        } catch (_) {}

        try {
            const pubStake = await staking.publisherStakes(addr);
            if (pubStake > 0n) {
                console.log(`  Publisher Stake   : \x1b[32m${ethers.formatEther(pubStake)} tCREG\x1b[0m`);
            }
        } catch (_) {}

        console.log('');
    } catch (err) {
        console.error(`\x1b[31m[Error querying status: ${err.message}]\x1b[0m`);
    }
}

main().catch(err => {
    console.error('\x1b[31m[Execution Error]\x1b[0m', err);
    process.exit(1);
});
