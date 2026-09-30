#!/usr/bin/env node
// Real Sepolia publisher stake and package-finalization relay authorization.
const fs = require('node:fs');
const path = require('node:path');
const { ethers } = require('../../scripts/evm-tool/node_modules/ethers');

const deployment = JSON.parse(fs.readFileSync(path.resolve(__dirname, 'deployment-manifest.json'), 'utf8'));
const provider = new ethers.JsonRpcProvider(process.env.SEPOLIA_RPC_URL || 'https://ethereum-sepolia-rpc.publicnode.com');
const bridgeAddress = '0xb0cBe3b30F05c60E5BFD122F57F19C20826C45EE';
const publisherAddress = '0xa40D79527314efD4964D49c4a0673C87b612348B';
const gasTarget = ethers.parseEther('0.01');
const publisherStake = ethers.parseEther('10');

function wallet(name, address) {
  let result;
  try { result = new ethers.Wallet((process.env[name] || '').trim(), provider); }
  catch { throw new Error(`Could not load ${name}`); }
  if (result.address.toLowerCase() !== address.toLowerCase()) throw new Error(`${name} address mismatch`);
  return result;
}

async function waitFor(tx, stage) {
  console.log(`${stage}_transaction=${tx.hash}`);
  const receipt = await tx.wait(1);
  if (!receipt || receipt.status !== 1) throw new Error(`${stage} failed`);
}

async function main() {
  if ((await provider.getNetwork()).chainId !== 11155111n) throw new Error('Expected live Sepolia');
  const treasury = wallet('CREG_PHASE_C_TREASURY_KEY', '0x3A3206D683441AA91cCA7CD5757B1F0ca463213A');
  const publisher = wallet('CREG_PHASE_C_PUBLISHER_KEY', publisherAddress);
  const gov1 = wallet('CREG_PHASE_C_GOV_1_KEY', '0x6323F9780Edf77763CF77401e559711372D76b8C');
  const gov2 = wallet('CREG_PHASE_C_GOV_2_KEY', '0x5dBD86B868F576e0C5b6dee9f9700A650FCFF639');
  const token = new ethers.Contract(deployment.cregToken, [
    'function balanceOf(address) view returns (uint256)',
    'function allowance(address,address) view returns (uint256)',
    'function transfer(address,uint256) returns (bool)',
    'function approve(address,uint256) returns (bool)',
  ], provider);
  const staking = new ethers.Contract(deployment.staking, [
    'function activeValidatorCount() view returns (uint256)',
    'function minPublisherStake() view returns (uint256)',
    'function publisherStakes(address) view returns (uint256)',
    'function stakeAsPublisher(uint256)',
  ], provider);
  const registry = new ethers.Contract(deployment.registry, [
    'function packageFinalizeRelays(address) view returns (bool)',
    'function enforceFinalizeRelays() view returns (bool)',
    'function setPackageFinalizeRelay(address,bool)',
  ], provider);
  const governance = new ethers.Contract(deployment.governance, [
    'function threshold() view returns (uint256)',
    'function proposalCount() view returns (uint256)',
    'function isSigner(address) view returns (bool)',
    'function submit(address,bytes,string) returns (uint256)',
    'function vote(uint256,bool)',
  ], provider);
  if ((await staking.activeValidatorCount()) !== 4n ||
      (await staking.minPublisherStake()) !== ethers.parseEther('1') ||
      (await governance.threshold()) !== 2n ||
      !await governance.isSigner(gov1.address) || !await governance.isSigner(gov2.address) ||
      !await registry.enforceFinalizeRelays()) {
    throw new Error('Real package-service prerequisites differ from the reviewed deployment');
  }
  console.log(JSON.stringify({ network: 'sepolia', registry: deployment.registry,
    bridge: bridgeAddress, publisher: publisherAddress,
    publisher_stake_wei: String(await staking.publisherStakes(publisherAddress)),
    relay_authorized: await registry.packageFinalizeRelays(bridgeAddress),
    planned_publisher_stake_wei: String(publisherStake),
    broadcast: process.argv.includes('--broadcast') }, null, 2));
  if (!process.argv.includes('--broadcast')) return;

  const publisherBalance = await token.balanceOf(publisherAddress);
  const staked = await staking.publisherStakes(publisherAddress);
  if (staked === 0n && publisherBalance < publisherStake) {
    await waitFor(await token.connect(treasury).transfer(publisherAddress, publisherStake - publisherBalance), 'publisher_creg_funding');
  }
  for (const [name, address] of [['publisher', publisherAddress], ['bridge', bridgeAddress]]) {
    const balance = await provider.getBalance(address);
    if (balance < gasTarget) {
      await waitFor(await treasury.sendTransaction({ to: address, value: gasTarget - balance }), `${name}_gas_funding`);
    }
  }
  if (staked === 0n) {
    if ((await token.allowance(publisherAddress, deployment.staking)) < publisherStake) {
      await waitFor(await token.connect(publisher).approve(deployment.staking, publisherStake), 'publisher_stake_allowance');
    }
    await waitFor(await staking.connect(publisher).stakeAsPublisher(publisherStake), 'publisher_stake');
  }
  if ((await staking.publisherStakes(publisherAddress)) < publisherStake) throw new Error('Publisher stake not live');

  if (!await registry.packageFinalizeRelays(bridgeAddress)) {
    const proposalId = await governance.proposalCount();
    const data = registry.interface.encodeFunctionData('setPackageFinalizeRelay', [bridgeAddress, true]);
    await waitFor(await governance.connect(gov1).submit(deployment.registry, data, `Phase C authorize package relay ${bridgeAddress}`), 'relay_governance_submit');
    await waitFor(await governance.connect(gov1).vote(proposalId, true), 'relay_governance_vote_1');
    await waitFor(await governance.connect(gov2).vote(proposalId, true), 'relay_governance_vote_2');
  }
  if (!await registry.packageFinalizeRelays(bridgeAddress)) throw new Error('Finalize relay not active');
  console.log('publisher_staked=true finalize_relay_authorized=true');
}

main().catch(() => { console.error('Phase C service bootstrap stopped; inspect public transactions and on-chain state.'); process.exitCode = 1; });
