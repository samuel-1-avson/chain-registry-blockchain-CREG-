#!/usr/bin/env node
// Read-only Sepolia snapshot for the planned real Phase C validator roster.
const fs = require('node:fs');
const path = require('node:path');
const { ethers } = require('../../scripts/evm-tool/node_modules/ethers');

const testnet = path.resolve(__dirname, '..');
const spec = JSON.parse(fs.readFileSync(path.join(testnet, 'chain-spec.sepolia.json'), 'utf8'));
const manifest = JSON.parse(fs.readFileSync(path.join(testnet, 'archive/fresh-testnet-key-manifest-2026-08-01.json'), 'utf8'));
const deploymentPath = process.argv[2];
if (deploymentPath) {
  const deployment = JSON.parse(fs.readFileSync(path.resolve(deploymentPath), 'utf8'));
  if (deployment.chainId !== '11155111') throw new Error('Expected a Sepolia deployment manifest');
  Object.assign(spec.contracts, {
    staking: deployment.staking, creg_token: deployment.cregToken,
    governance: deployment.governance, validator_rewards: deployment.validatorRewards,
  });
}
const rpcUrl = process.env.SEPOLIA_RPC_URL || 'https://ethereum-sepolia-rpc.publicnode.com';
const provider = new ethers.JsonRpcProvider(rpcUrl);
const staking = new ethers.Contract(spec.contracts.staking, [
  'function validators(address) view returns (uint256 stake, uint8 state, uint256 unbondingAt, uint256 slashCount, uint256 ejectedAt, uint256 appliedAt)',
  'function activeValidatorCount() view returns (uint256)',
  'function minValidatorStake() view returns (uint256)',
  'function validatorSetEpoch() view returns (uint256)',
  'function emergencyGovernanceEnabled() view returns (bool)',
  'function governance() view returns (address)',
], provider);
const token = new ethers.Contract(spec.contracts.creg_token, [
  'function balanceOf(address) view returns (uint256)',
  'function allowance(address,address) view returns (uint256)',
  'function owner() view returns (address)',
  'function treasury() view returns (address)',
], provider);
const governance = new ethers.Contract(spec.contracts.governance, [
  'function threshold() view returns (uint256)',
  'function signerCount() view returns (uint256)',
  'function signers(uint256) view returns (address)',
  'function proposalCount() view returns (uint256)',
], provider);
const rewards = new ethers.Contract(spec.contracts.validator_rewards, [
  'function treasury() view returns (address)',
  'function governance() view returns (address)',
], provider);

async function main() {
  const network = await provider.getNetwork();
  if (network.chainId !== 11155111n) throw new Error(`Expected Sepolia, got ${network.chainId}`);
  const head = await provider.getBlockNumber();
  const blockTag = head - Number(spec.l1.min_finality_blocks);
  if (blockTag < 0) throw new Error('Sepolia head is below required finality depth');
  const at = { blockTag };
  const [activeCount, minStake, epoch, emergencyGovernance, stakingGovernance, tokenOwner, treasury, threshold, signerCount, proposalCount, rewardsTreasury, rewardsGovernance] = await Promise.all([
    staking.activeValidatorCount(at), staking.minValidatorStake(at), staking.validatorSetEpoch(at),
    staking.emergencyGovernanceEnabled(at), staking.governance(at), token.owner(at),
    token.treasury(at), governance.threshold(at), governance.signerCount(at), governance.proposalCount(at),
    rewards.treasury(at), rewards.governance(at),
  ]);
  if (stakingGovernance.toLowerCase() !== spec.contracts.governance.toLowerCase()) {
    throw new Error('Live staking governance differs from the chain spec');
  }
  const signers = [];
  for (let i = 0; i < Number(signerCount); i++) {
    const address = await governance.signers(i, at);
    signers.push({ address, eth_wei: String(await provider.getBalance(address, blockTag)) });
  }
  const addresses = [
    ...manifest.validators.map(v => ({ id: v.id, address: v.evm_address, roster: 'planned' })),
    ...(!deploymentPath ? spec.validator_set.validators.map(v => ({ id: v.id, address: v.eth_address, roster: 'old-spec' })) : []),
  ];
  const validators = [];
  for (const item of addresses) {
    const entry = await staking.validators(item.address, at);
    const [creg, eth] = await Promise.all([
      token.balanceOf(item.address, at), provider.getBalance(item.address, blockTag),
    ]);
    validators.push({
      ...item, state: Number(entry.state), stake_wei: String(entry.stake),
      creg_balance_wei: String(creg), eth_balance_wei: String(eth),
    });
  }
  console.log(JSON.stringify({
    network: 'sepolia', head, block: blockTag,
    deployment_manifest: deploymentPath || null,
    staking: spec.contracts.staking, active_validator_count: String(activeCount),
    min_validator_stake_wei: String(minStake), validator_set_epoch: String(epoch),
    emergency_governance_enabled: emergencyGovernance,
    token: spec.contracts.creg_token, token_owner: tokenOwner, treasury,
    treasury_creg_balance_wei: String(await token.balanceOf(treasury, at)),
    validator_rewards: spec.contracts.validator_rewards,
    rewards_treasury: rewardsTreasury, rewards_governance: rewardsGovernance,
    treasury_rewards_allowance_wei: String(await token.allowance(rewardsTreasury, spec.contracts.validator_rewards, at)),
    governance: stakingGovernance, governance_threshold: String(threshold),
    governance_proposal_count: String(proposalCount), signers,
    validators,
  }, null, 2));
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
