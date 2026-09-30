// Executes only the reviewed 10,000 tCREG Sepolia funding proposal.
// Signing material stays in memory and is never printed or written to evidence.
const fs = require('node:fs');
const { spawnSync } = require('node:child_process');
const { JsonRpcProvider, Contract, Wallet, parseUnits, formatEther, keccak256 } = require('../../scripts/evm-tool/node_modules/ethers');

async function main() {
  const [proposalPath, evidencePath] = process.argv.slice(2);
  if (!proposalPath || !evidencePath) throw new Error('Provide proposal and new evidence paths');
  if (fs.existsSync(evidencePath)) throw new Error('Execution evidence already exists; inspect the transaction before retrying');
  const proposal = JSON.parse(fs.readFileSync(proposalPath, 'utf8'));
  if (proposal.chain_id !== 11155111 || proposal.token_amount !== '10000' || proposal.gas_budget_eth !== '0.001' || proposal.recipient !== '0x721C8dBD8208CA9dBdcC12AE9339225458608B81' || proposal.token !== '0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6') throw new Error('Proposal differs from user-approved bounds');
  const provider = new JsonRpcProvider(proposal.rpc);
  if ((await provider.getNetwork()).chainId !== 11155111n) throw new Error('Not Sepolia');
  const secret = spawnSync(process.platform === 'win32' ? 'gcloud.cmd' : 'gcloud', [
    'secrets', 'versions', 'access', 'latest', '--secret=' + proposal.signing_secret_name,
    '--project=' + proposal.secret_project, '--quiet',
  ], { encoding: 'utf8', shell: process.platform === 'win32', windowsHide: true });
  if (secret.status !== 0 || !secret.stdout.trim()) throw new Error('Treasury secret access failed');
  const raw = secret.stdout.trim();
  const key = raw.startsWith('{') ? JSON.parse(raw).private_key : raw;
  const signer = new Wallet(key, provider);
  if (signer.address.toLowerCase() !== proposal.treasury.toLowerCase()) throw new Error('Signing key is not the reviewed treasury');
  const token = new Contract(proposal.token, ['function owner() view returns(address)', 'function treasury() view returns(address)', 'function balanceOf(address) view returns(uint256)', 'function transfer(address,uint256) returns(bool)'], signer);
  const [code, owner, treasury, reserve, before, nonce, fees, balance] = await Promise.all([
    provider.getCode(proposal.token), token.owner(), token.treasury(), token.balanceOf(signer.address),
    token.balanceOf(proposal.recipient), provider.getTransactionCount(signer.address, 'pending'), provider.getFeeData(), provider.getBalance(signer.address),
  ]);
  if (keccak256(code) !== proposal.token_code_keccak256 || owner !== proposal.token_owner || treasury !== proposal.treasury) throw new Error('Contract identity changed');
  if (nonce !== proposal.transaction.nonce || before !== 0n) throw new Error('Nonce or faucet balance changed; prepare a new reviewed proposal');
  const amount = BigInt(proposal.token_amount_atomic);
  if (reserve < amount) throw new Error('Insufficient reserve');
  const transaction = { to: proposal.token, data: proposal.transaction.data, value: 0n, chainId: 11155111, nonce };
  const gas = await provider.estimateGas({ ...transaction, from: signer.address });
  transaction.gasLimit = (gas * 125n + 99n) / 100n;
  transaction.maxFeePerGas = fees.maxFeePerGas;
  transaction.maxPriorityFeePerGas = fees.maxPriorityFeePerGas;
  const cap = parseUnits('0.001', 18);
  if (!transaction.maxFeePerGas || transaction.maxPriorityFeePerGas == null || transaction.gasLimit * transaction.maxFeePerGas > cap || balance < cap) throw new Error('Gas exceeds approved bounds');
  const evidence = { timestamp_utc: new Date().toISOString(), approval: 'User approved exact 10,000 tCREG transfer and maximum 0.001 Sepolia ETH gas', chain_id: 11155111, token: proposal.token, from: signer.address, recipient: proposal.recipient, amount: proposal.token_amount, nonce, gas_limit: transaction.gasLimit.toString(), max_fee_per_gas: transaction.maxFeePerGas.toString(), status: 'ready_to_broadcast' };
  const save = () => fs.writeFileSync(evidencePath, JSON.stringify(evidence, null, 2) + '\n');
  save();
  const sent = await signer.sendTransaction(transaction);
  evidence.transaction_hash = sent.hash;
  evidence.status = 'broadcast_receipt_pending';
  save();
  console.log(JSON.stringify(evidence));
  const receipt = await sent.wait(2, 180000);
  if (!receipt || receipt.status !== 1) throw new Error('Funding did not succeed; inspect saved transaction hash');
  const after = await token.balanceOf(proposal.recipient);
  if (after !== before + amount) throw new Error('Recipient balance does not match funding');
  Object.assign(evidence, { status: 'confirmed', block_number: receipt.blockNumber, confirmations: 2, gas_used: receipt.gasUsed.toString(), gas_cost_eth: formatEther(receipt.gasUsed * receipt.gasPrice), recipient_tokens_atomic: after.toString() });
  save();
  console.log(JSON.stringify(evidence));
}
main().catch(error => { console.error('Funding stopped:', error.shortMessage || error.message); process.exitCode = 1; });
