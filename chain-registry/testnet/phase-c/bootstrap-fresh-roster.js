#!/usr/bin/env node
// Fund and admit the four real Phase C validator wallets on fresh Sepolia contracts.
const fs = require('node:fs');
const path = require('node:path');
const { ethers } = require('../../scripts/evm-tool/node_modules/ethers');

const deployment = JSON.parse(fs.readFileSync(path.resolve(__dirname, 'deployment-manifest.json'), 'utf8'));
const manifest = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../archive/fresh-testnet-key-manifest-2026-08-01.json'), 'utf8'));
const provider = new ethers.JsonRpcProvider(process.env.SEPOLIA_RPC_URL || 'https://ethereum-sepolia-rpc.publicnode.com');
const tokenAmount = ethers.parseEther('100');
const gasTarget = ethers.parseEther('0.01');
const expectedTreasury = '0x3A3206D683441AA91cCA7CD5757B1F0ca463213A';
const expectedGovernanceSigners = [
  '0x6323F9780Edf77763CF77401e559711372D76b8C',
  '0x5dBD86B868F576e0C5b6dee9f9700A650FCFF639',
];

function walletFromEnv(name, expectedAddress) {
  let wallet;
  try { wallet = new ethers.Wallet((process.env[name] || '').trim(), provider); }
  catch { throw new Error(`Could not load ${name}`); }
  if (wallet.address.toLowerCase() !== expectedAddress.toLowerCase()) {
    throw new Error(`${name} does not match its public custody manifest`);
  }
  return wallet;
}

function logTransaction(stage, tx) { console.log(`${stage}_transaction=${tx.hash}`); }

async function waitFor(tx, stage) {
  logTransaction(stage, tx);
  const receipt = await tx.wait(1);
  if (!receipt || receipt.status !== 1) throw new Error(`${stage} transaction failed`);
}

async function main() {
  if ((await provider.getNetwork()).chainId !== 11155111n) throw new Error('Expected live Sepolia');
  const treasury = walletFromEnv('CREG_PHASE_C_TREASURY_KEY', expectedTreasury);
  const validators = manifest.validators.map((v, i) => ({
    ...v, wallet: walletFromEnv(`CREG_PHASE_C_VALIDATOR_${i + 1}_KEY`, v.evm_address),
  }));
  const gov1 = walletFromEnv('CREG_PHASE_C_GOV_1_KEY', expectedGovernanceSigners[0]);
  const gov2 = walletFromEnv('CREG_PHASE_C_GOV_2_KEY', expectedGovernanceSigners[1]);
  const token = new ethers.Contract(deployment.cregToken, [
    'function balanceOf(address) view returns (uint256)',
    'function allowance(address,address) view returns (uint256)',
    'function transfer(address,uint256) returns (bool)',
    'function approve(address,uint256) returns (bool)',
  ], provider);
  const staking = new ethers.Contract(deployment.staking, [
    'function validators(address) view returns (uint256 stake,uint8 state,uint256 unbondingAt,uint256 slashCount,uint256 ejectedAt,uint256 appliedAt)',
    'function minValidatorStake() view returns (uint256)',
    'function activeValidatorCount() view returns (uint256)',
    'function emergencyGovernanceEnabled() view returns (bool)',
    'function applyToBeValidator(uint256)',
    'function approveValidator(address)',
  ], provider);
  const governance = new ethers.Contract(deployment.governance, [
    'function threshold() view returns (uint256)',
    'function isSigner(address) view returns (bool)',
    'function proposalCount() view returns (uint256)',
    'function submit(address,bytes,string) returns (uint256)',
    'function vote(uint256,bool)',
  ], provider);
  if ((await staking.minValidatorStake()) !== tokenAmount ||
      (await governance.threshold()) !== 2n ||
      !await governance.isSigner(gov1.address) || !await governance.isSigner(gov2.address) ||
      !await staking.emergencyGovernanceEnabled()) {
    throw new Error('Fresh contract admission prerequisites differ from the reviewed deployment');
  }
  const activeBefore = await staking.activeValidatorCount();
  const treasuryBalance = await token.balanceOf(treasury.address);
  const treasuryEth = await provider.getBalance(treasury.address);
  console.log(JSON.stringify({
    network: 'sepolia', staking: deployment.staking, governance: deployment.governance,
    token: deployment.cregToken, treasury: treasury.address,
    treasury_creg_wei: String(treasuryBalance), treasury_eth_wei: String(treasuryEth),
    active_before: String(activeBefore), required_active: 4,
    planned_validator_stake_each_wei: String(tokenAmount),
    planned_gas_target_each_wei: String(gasTarget),
    broadcast: process.argv.includes('--broadcast'),
  }, null, 2));
  if (!process.argv.includes('--broadcast')) return;
  if (activeBefore > 4n) throw new Error('Fresh staking contract has unexpected active validators');
  if (treasuryBalance < tokenAmount * 4n || treasuryEth < ethers.parseEther('0.10')) {
    throw new Error('Treasury has insufficient real CREG or Sepolia ETH for bootstrap');
  }

  for (const v of validators) {
    const entry = await staking.validators(v.wallet.address);
    if (Number(entry.state) !== 0 && Number(entry.state) !== 1 && Number(entry.state) !== 2) {
      throw new Error(`Unexpected staking state for ${v.id}`);
    }
    if (Number(entry.state) === 0) {
      const balance = await token.balanceOf(v.wallet.address);
      if (balance < tokenAmount) {
        await waitFor(await token.connect(treasury).transfer(v.wallet.address, tokenAmount - balance), `${v.id}_creg_funding`);
      }
    }
    const ethBalance = await provider.getBalance(v.wallet.address);
    if (ethBalance < gasTarget) {
      await waitFor(await treasury.sendTransaction({ to: v.wallet.address, value: gasTarget - ethBalance }), `${v.id}_gas_funding`);
    }
  }
  for (const [i, signer] of [gov1, gov2, { address: '0x3Febc440Bb85AD51B83576b3Dcc53BD253D75cCc' }].entries()) {
    const ethBalance = await provider.getBalance(signer.address);
    if (ethBalance < gasTarget) {
      await waitFor(await treasury.sendTransaction({ to: signer.address, value: gasTarget - ethBalance }), `governance_${i + 1}_gas_funding`);
    }
  }

  for (const v of validators) {
    const entry = await staking.validators(v.wallet.address);
    if (Number(entry.state) === 2) continue;
    if (Number(entry.state) === 0) {
      if ((await token.allowance(v.wallet.address, deployment.staking)) < tokenAmount) {
        await waitFor(await token.connect(v.wallet).approve(deployment.staking, tokenAmount), `${v.id}_stake_allowance`);
      }
      await waitFor(await staking.connect(v.wallet).applyToBeValidator(tokenAmount), `${v.id}_application`);
    }
    const pending = await staking.validators(v.wallet.address);
    if (Number(pending.state) !== 1 || pending.stake !== tokenAmount) {
      throw new Error(`${v.id} failed to enter Pending with 100 CREG escrowed`);
    }
  }

  for (const v of validators) {
    if (Number((await staking.validators(v.wallet.address)).state) === 2) continue;
    const data = staking.interface.encodeFunctionData('approveValidator', [v.wallet.address]);
    const proposalId = await governance.proposalCount();
    await waitFor(await governance.connect(gov1).submit(deployment.staking, data, `Phase C admit ${v.wallet.address}`), `${v.id}_governance_submit`);
    await waitFor(await governance.connect(gov1).vote(proposalId, true), `${v.id}_governance_vote_1`);
    await waitFor(await governance.connect(gov2).vote(proposalId, true), `${v.id}_governance_vote_2`);
    const active = await staking.validators(v.wallet.address);
    if (Number(active.state) !== 2 || active.stake !== tokenAmount) {
      throw new Error(`${v.id} was not activated by the 2-of-3 governance approval`);
    }
    console.log(`${v.id}_active=true`);
  }
  if ((await staking.activeValidatorCount()) !== 4n) throw new Error('Fresh L1 active set is not exactly four');
  console.log('active_validator_count=4');
}

main().catch(() => { console.error('Phase C roster bootstrap stopped; inspect public transaction hashes and on-chain state.'); process.exitCode = 1; });
