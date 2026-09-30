#!/usr/bin/env node
// One-time Sepolia rescue of the wallet exposed by a parser error.
const { ethers } = require('../../scripts/evm-tool/node_modules/ethers');
const source = '0x111b4B1Ad99bC127224dFBaB620b4Ee3A34569e8';
const destination = '0x3A3206D683441AA91cCA7CD5757B1F0ca463213A';

async function main() {
  const provider = new ethers.JsonRpcProvider('https://ethereum-sepolia-rpc.publicnode.com');
  if ((await provider.getNetwork()).chainId !== 11155111n) throw new Error('Wrong chain');
  let wallet;
  try { wallet = new ethers.Wallet((process.env.CREG_EXPOSED_DEPLOYER_KEY || '').trim(), provider); }
  catch { throw new Error('Exposed wallet key could not be parsed'); }
  if (wallet.address.toLowerCase() !== source.toLowerCase()) throw new Error('Unexpected source wallet');
  if ((await provider.getCode(destination)) !== '0x') throw new Error('Unexpected destination contract code');
  const balance = await provider.getBalance(source);
  const fee = await provider.getFeeData();
  if (!fee.maxFeePerGas || !fee.maxPriorityFeePerGas) throw new Error('Sepolia fee data unavailable');
  const maxFeePerGas = fee.maxFeePerGas * 2n;
  const reserve = 21000n * maxFeePerGas;
  if (balance <= reserve) throw new Error('No transferable balance remains');
  const value = balance - reserve;
  console.log(JSON.stringify({ source, destination, source_balance_wei: String(balance),
    transfer_wei: String(value), broadcast: process.argv.includes('--broadcast') }));
  if (!process.argv.includes('--broadcast')) return;
  const tx = await wallet.sendTransaction({
    to: destination, value, gasLimit: 21000n,
    maxFeePerGas, maxPriorityFeePerGas: fee.maxPriorityFeePerGas,
  });
  console.log(`rescue_transaction=${tx.hash}`);
  const receipt = await tx.wait(6);
  if (!receipt || receipt.status !== 1) throw new Error('Rescue transaction did not finalize');
  console.log(`destination_balance_wei=${await provider.getBalance(destination)}`);
}

main().catch(() => { console.error('Exposed-wallet rescue failed; inspect balances and transaction hash.'); process.exitCode = 1; });
