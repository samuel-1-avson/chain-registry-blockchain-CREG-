// Read-only inventory. No keys, signer, broadcast, or funding mutation.
const fs = require('node:fs');
const path = require('node:path');
const { JsonRpcProvider, Contract, formatUnits, formatEther, keccak256 } = require('../../scripts/evm-tool/node_modules/ethers');

async function main() {
  const output = process.argv[2];
  if (!output) throw new Error('Provide an evidence output path');
  const manifest = JSON.parse(fs.readFileSync(path.join(__dirname, 'deployment-manifest.json'), 'utf8'));
  const spec = JSON.parse(fs.readFileSync(path.join(__dirname, 'r7/chain-spec.json'), 'utf8'));
  const provider = new JsonRpcProvider('https://api.testnet.cregnet.dev/sepolia');
  const network = await provider.getNetwork();
  if (network.chainId !== 11155111n) throw new Error('Expected Sepolia');
  const block = await provider.getBlockNumber();
  const read = { blockTag: block };
  const token = new Contract(manifest.cregToken, [
    'function owner() view returns(address)', 'function treasury() view returns(address)',
    'function balanceOf(address) view returns(uint256)',
    'function allowance(address,address) view returns(uint256)',
  ], provider);
  const staking = new Contract(manifest.staking, [
    'function governance() view returns(address)', 'function owner() view returns(address)',
    'function publisherStakes(address) view returns(uint256)',
    'function isActiveValidator(address) view returns(bool)',
  ], provider);
  const rewards = new Contract(manifest.validatorRewards, [
    'function governance() view returns(address)', 'function treasury() view returns(address)',
    'function staking() view returns(address)', 'function cregToken() view returns(address)',
    'function totalRewardsDistributed() view returns(uint256)',
    'function totalPackagesVerified() view returns(uint256)',
    'function pendingRewards(address) view returns(uint256)',
    'function packagesVerified(address) view returns(uint256)',
  ], provider);
  const [tokenOwner, treasury, stakingOwner, stakingGovernance, rewardsGovernance,
    rewardsTreasury, rewardsStaking, rewardsToken, distributed, packageCount] = await Promise.all([
    token.owner(read), token.treasury(read), staking.owner(read), staking.governance(read),
    rewards.governance(read), rewards.treasury(read), rewards.staking(read), rewards.cregToken(read),
    rewards.totalRewardsDistributed(read), rewards.totalPackagesVerified(read),
  ]);
  if (tokenOwner.toLowerCase() !== manifest.governance.toLowerCase() ||
      treasury.toLowerCase() !== manifest.deployer.toLowerCase() ||
      rewardsToken.toLowerCase() !== manifest.cregToken.toLowerCase() ||
      rewardsStaking.toLowerCase() !== manifest.staking.toLowerCase()) throw new Error('Manifest/contract mismatch');
  const balances = {};
  for (const [name, address] of Object.entries({treasury, faucet: '0x721C8dBD8208CA9dBdcC12AE9339225458608B81', validator_rewards: manifest.validatorRewards})) {
    const [creg, eth, code] = await Promise.all([token.balanceOf(address, read), provider.getBalance(address, block), provider.getCode(address, block)]);
    balances[name] = { address, creg: formatUnits(creg, 18), sepolia_eth: formatEther(eth), code_keccak256: code === '0x' ? null : keccak256(code) };
  }
  const validators = [];
  for (const validator of spec.validator_set.validators) {
    const address = validator.eth_address;
    const [active, stake, pending, packages] = await Promise.all([
      staking.isActiveValidator(address, read), staking.publisherStakes(address, read),
      rewards.pendingRewards(address, read), rewards.packagesVerified(address, read),
    ]);
    validators.push({ id: validator.id, address, active_on_l1: active, publisher_stake_creg: formatUnits(stake, 18), pending_reward_creg: formatUnits(pending, 18), reward_packages: packages.toString() });
  }
  const report = {
    captured_at_utc: new Date().toISOString(), status: 'read_only_inventory', chain_id: 11155111, block,
    rpc: provider._getConnection().url,
    roles: { token_owner: tokenOwner, treasury, staking_owner: stakingOwner, staking_governance: stakingGovernance,
      rewards_governance: rewardsGovernance, rewards_treasury: rewardsTreasury, rewards_staking: rewardsStaking, rewards_token: rewardsToken },
    balances, validators, rewards_total_distributed_creg: formatUnits(distributed, 18),
    treasury_allowance_to_rewards_creg: formatUnits(await token.allowance(treasury, manifest.validatorRewards, read), 18),
    rewards_total_packages: packageCount.toString(), signing_or_funding_performed: false,
  };
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
}
main().catch(error => { console.error(error.shortMessage || error.message); process.exitCode = 1; });
