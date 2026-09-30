#!/usr/bin/env node
// ==============================================================================
// Chain Registry (CREG) — 2-of-3 Multisig Governance Validator Approval (Sepolia)
// ==============================================================================

const path = require('path');
const fs = require('fs');

let ethers;
const candidatePaths = [
    '../scripts/evm-tool/node_modules/ethers',
    './scripts/evm-tool/node_modules/ethers',
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
    console.error('Failed to load ethers');
    process.exit(1);
}

const RPC_URL = process.env.SEPOLIA_RPC_URL || 'https://explorer.testnet.cregnet.dev/rpc';
const GOV_ADDR = '0x122Bc2C704B966C74004506F98F13bf7b5F75b9D';
const STAKING_ADDR = '0x30604Aa6f763fbde6aDCffbd8960af3031037331';

// Supply two distinct, current governance signer keys at runtime. Never commit them.
const SIGNER1_KEY = process.env.CREG_GOV_SIGNER1_KEY;
const SIGNER2_KEY = process.env.CREG_GOV_SIGNER2_KEY;

const APPLICANT_ADDR = process.argv[2] || '0x5f424b4ec88073fd461eb194833681a31adfa311';

const GOV_ABI = [
    'function threshold() view returns (uint256)',
    'function proposalCount() view returns (uint256)',
    'function isSigner(address) view returns (bool)',
    'function submit(address target, bytes callData, string description) returns (uint256 id)',
    'function vote(uint256 id, bool approve)',
    'function getProposal(uint256 id) view returns (address target, string description, uint256 submittedAt, uint8 status, uint256 approvalCount, uint256 rejectionCount)'
];

const STAKING_ABI = [
    'function approveValidator(address validator) external',
    'function validators(address) view returns (uint256 stake, uint8 state, uint256 unbondingAt, uint256 slashCount, uint256 ejectedAt, uint256 appliedAt)',
    'function activeValidatorCount() view returns (uint256)',
    'function emergencyGovernanceEnabled() view returns (bool)'
];

async function main() {
    if (!/^0x[0-9a-fA-F]{64}$/.test(SIGNER1_KEY || '') ||
        !/^0x[0-9a-fA-F]{64}$/.test(SIGNER2_KEY || '')) {
        throw new Error('Set CREG_GOV_SIGNER1_KEY and CREG_GOV_SIGNER2_KEY to two current governance signer keys.');
    }
    console.log('\x1b[36m============================================================\x1b[0m');
    console.log('\x1b[36m   CREG Governance Multi-Sig Validator Approval (Sepolia)   \x1b[0m');
    console.log('\x1b[36m============================================================\x1b[0m');
    console.log(`  Target Applicant: \x1b[32m${APPLICANT_ADDR}\x1b[0m`);
    console.log(`  Staking Hub     : \x1b[90m${STAKING_ADDR}\x1b[0m`);
    console.log(`  Governance      : \x1b[90m${GOV_ADDR}\x1b[0m`);
    console.log(`  Sepolia RPC     : \x1b[90m${RPC_URL}\x1b[0m\n`);

    const provider = new ethers.JsonRpcProvider(RPC_URL);
    const signer1 = new ethers.Wallet(SIGNER1_KEY, provider);
    const signer2 = new ethers.Wallet(SIGNER2_KEY, provider);
    if (signer1.address === signer2.address) throw new Error('Governance signer keys must be distinct.');

    const gov = new ethers.Contract(GOV_ADDR, GOV_ABI, provider);
    const staking = new ethers.Contract(STAKING_ADDR, STAKING_ABI, provider);
    if (!await gov.isSigner(signer1.address) || !await gov.isSigner(signer2.address)) {
        throw new Error('Both runtime keys must belong to current governance signers.');
    }

    // 1. Verify current applicant state
    const entry = await staking.validators(APPLICANT_ADDR);
    const state = Number(entry.state);
    const stateNames = { 0: 'None', 1: 'Pending', 2: 'Active', 3: 'Unbonding', 4: 'Ejected' };
    console.log(`  Current On-Chain State: \x1b[33m${stateNames[state]} (${state})\x1b[0m`);
    console.log(`  Staked Amount         : \x1b[32m${ethers.formatEther(entry.stake)} tCREG\x1b[0m`);

    if (state === 2) {
        console.log('\x1b[32m[Notice] Validator is ALREADY Active on Sepolia L1!\x1b[0m');
        process.exit(0);
    }
    if (state !== 1) {
        console.error(`\x1b[31m[Error] Applicant must be in Pending state (1); current state is ${state}\x1b[0m`);
        process.exit(1);
    }

    // 2. Check Gas balances
    const s1Bal = await provider.getBalance(signer1.address);
    const s2Bal = await provider.getBalance(signer2.address);
    console.log(`\n  Signer 1 (${signer1.address}): ${ethers.formatEther(s1Bal)} ETH`);
    console.log(`  Signer 2 (${signer2.address}): ${ethers.formatEther(s2Bal)} ETH`);

    if (s2Bal < ethers.parseEther('0.005')) {
        console.log('\x1b[33m  Funding Signer 2 with 0.015 ETH from Signer 1 for gas...\x1b[0m');
        const fundTx = await signer1.sendTransaction({
            to: signer2.address,
            value: ethers.parseEther('0.015')
        });
        console.log(`  Funding Tx: \x1b[90m${fundTx.hash}\x1b[0m`);
        await fundTx.wait(1);
        console.log('\x1b[32m  ✓ Signer 2 funded!\x1b[0m');
    }

    // 3. Prepare Calldata for approveValidator(applicant)
    const stakingIface = new ethers.Interface(STAKING_ABI);
    const calldata = stakingIface.encodeFunctionData('approveValidator', [APPLICANT_ADDR]);
    const description = `Approve external validator ${APPLICANT_ADDR}`;

    const govSigner1 = gov.connect(signer1);
    const govSigner2 = gov.connect(signer2);

    // 4. Submit proposal
    const nextPropId = Number(await gov.proposalCount());
    console.log(`\n\x1b[33m[Step 1/3] Submitting Governance Proposal #${nextPropId} by Signer 1...\x1b[0m`);
    const submitTx = await govSigner1.submit(STAKING_ADDR, calldata, description);
    console.log(`  Tx: \x1b[90m${submitTx.hash}\x1b[0m`);
    await submitTx.wait(1);
    console.log(`\x1b[32m  ✓ Proposal #${nextPropId} submitted!\x1b[0m`);

    // 5. Vote from Signer 1
    console.log(`\n\x1b[33m[Step 2/3] Signer 1 voting 'Approve' (1/2 threshold)...\x1b[0m`);
    const vote1Tx = await govSigner1.vote(nextPropId, true);
    console.log(`  Tx: \x1b[90m${vote1Tx.hash}\x1b[0m`);
    await vote1Tx.wait(1);
    console.log(`\x1b[32m  ✓ Signer 1 vote confirmed!\x1b[0m`);

    // 6. Vote from Signer 2 (triggers auto-execution)
    console.log(`\n\x1b[33m[Step 3/3] Signer 2 co-signing & voting 'Approve' (2/2 threshold -> AUTO-EXECUTE)...\x1b[0m`);
    const vote2Tx = await govSigner2.vote(nextPropId, true);
    console.log(`  Tx: \x1b[90m${vote2Tx.hash}\x1b[0m`);
    const receipt2 = await vote2Tx.wait(1);
    console.log(`\x1b[32m  ✓ Signer 2 vote confirmed in block ${receipt2.blockNumber}!\x1b[0m`);

    // 7. Verify final on-chain state
    const afterEntry = await staking.validators(APPLICANT_ADDR);
    const afterState = Number(afterEntry.state);
    const activeCount = await staking.activeValidatorCount();

    console.log('\n\x1b[32m============================================================\x1b[0m');
    console.log('\x1b[32m   ✓ VALIDATOR SUCCESSFULLY APPROVED ON SEPOLIA L1!        \x1b[0m');
    console.log('\x1b[32m============================================================\x1b[0m');
    console.log(`  Validator Address    : \x1b[32m${APPLICANT_ADDR}\x1b[0m`);
    console.log(`  L1 Validator State   : \x1b[32m${stateNames[afterState]} (State ${afterState})\x1b[0m`);
    console.log(`  Active L1 Validators : \x1b[32m${activeCount}\x1b[0m`);
    console.log(`  Execution Tx Hash    : \x1b[36mhttps://sepolia.etherscan.io/tx/${vote2Tx.hash}\x1b[0m\n`);
}

main().catch(err => {
    console.error('\x1b[31m[Governance Error]\x1b[0m', err);
    process.exit(1);
});
