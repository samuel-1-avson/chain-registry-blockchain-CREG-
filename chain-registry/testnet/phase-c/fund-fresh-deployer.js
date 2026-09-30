#!/usr/bin/env node
// Move Sepolia ETH from the legacy operator wallet to the new Phase C deployer.
const fs = require('node:fs');
const path = require('node:path');
const { ethers } = require('../../scripts/evm-tool/node_modules/ethers');

const legacyAddress = '0xb0935C6C7b9F024013eE13d9C194683199F07eCC';
const newDeployer = '0x111b4B1Ad99bC127224dFBaB620b4Ee3A34569e8';
const amount = ethers.parseEther('0.30');
const envPath = path.resolve(__dirname, '../.env.sepolia');
const rpcUrl = process.env.SEPOLIA_RPC_URL || 'https://ethereum-sepolia-rpc.publicnode.com';

function readLegacyKey() {
  const line = fs.readFileSync(envPath, 'utf8').split(/\r?\n/)
    .find(value => /^DEPLOYER_KEY=/.test(value));
  if (!line) throw new Error('Legacy DEPLOYER_KEY is unavailable');
  return line.slice('DEPLOYER_KEY='.length).trim().replace(/^['"]|['"]$/g, '');
}

async function main() {
  const provider = new ethers.JsonRpcProvider(rpcUrl);
  const network = await provider.getNetwork();
  if (network.chainId !== 11155111n) throw new Error('Funding requires real Sepolia');
  const signer = new ethers.Wallet(readLegacyKey(), provider);
  if (signer.address.toLowerCase() !== legacyAddress.toLowerCase()) {
    throw new Error('Legacy key does not match the expected old treasury/deployer');
  }
  const [oldBalance, newBalance, newCode] = await Promise.all([
    provider.getBalance(legacyAddress), provider.getBalance(newDeployer), provider.getCode(newDeployer),
  ]);
  if (newCode !== '0x') throw new Error('New deployer address unexpectedly contains contract code');
  console.log(JSON.stringify({
    network: 'sepolia', from: legacyAddress, to: newDeployer,
    planned_eth_wei: String(amount), from_eth_wei: String(oldBalance),
    to_eth_wei: String(newBalance), broadcast: process.argv.includes('--broadcast'),
  }, null, 2));
  if (!process.argv.includes('--broadcast')) return;
  if (newBalance !== 0n) throw new Error('New deployer is already funded; inspect before sending again');
  if (oldBalance < amount + ethers.parseEther('0.01')) {
    throw new Error('Legacy wallet cannot fund 0.30 ETH and retain a gas reserve');
  }
  const tx = await signer.sendTransaction({ to: newDeployer, value: amount });
  console.log(`funding_transaction=${tx.hash}`);
  const receipt = await tx.wait(6);
  if (!receipt || receipt.status !== 1) throw new Error('Funding transaction did not finalize');
  const fundedBalance = await provider.getBalance(newDeployer);
  if (fundedBalance < amount) throw new Error('New deployer balance is below the funded amount');
  console.log(`funded_balance_wei=${fundedBalance}`);
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
