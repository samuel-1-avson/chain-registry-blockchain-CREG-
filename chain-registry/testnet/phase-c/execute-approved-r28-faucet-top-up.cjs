// Broadcast only the separately approved 92,000 tCREG R28 treasury tranche.
// The signed hash is saved before broadcast; never retry an uncertain broadcast.
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const {
  JsonRpcProvider, Contract, Interface, Wallet, parseUnits, formatEther, keccak256,
} = require('../../scripts/evm-tool/node_modules/ethers');

async function main() {
  const [proposalPath, evidencePath] = process.argv.slice(2);
  if (!proposalPath || !evidencePath) throw new Error('Provide the approved proposal and a new evidence path');
  if (fs.existsSync(evidencePath)) throw new Error('Evidence already exists; inspect its transaction before retrying');
  const proposal = JSON.parse(fs.readFileSync(proposalPath, 'utf8'));
  const recipient = '0x721C8dBD8208CA9dBdcC12AE9339225458608B81';
  const tokenAddress = '0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6';
  const amount = parseUnits('92000', 18);
  const initialBalance = parseUnits('8000', 18);
  const finalBalance = parseUnits('100000', 18);
  const gasBudget = parseUnits('0.001', 18);
  const transferInterface = new Interface(['function transfer(address,uint256) returns(bool)']);
  const expectedData = transferInterface.encodeFunctionData('transfer', [recipient, amount]);
  if (proposal.status !== 'prepared_read_only_not_approved_not_signed_not_broadcast' ||
      proposal.chain_id !== 11155111 ||
      proposal.rpc_url !== 'https://api.testnet.cregnet.dev/sepolia' ||
      proposal.secret_project !== 'gen-lang-client-0022105784' ||
      proposal.signing_secret_name !== 'creg-phase-c-treasury-v2-evm' ||
      proposal.treasury.toLowerCase() !== '0x3a3206d683441aa91cca7cd5757b1f0ca463213a' ||
      proposal.token_contract.toLowerCase() !== tokenAddress.toLowerCase() ||
      proposal.faucet.toLowerCase() !== recipient.toLowerCase() ||
      proposal.top_up_amount_wei !== amount.toString() ||
      proposal.faucet_balance_before_tcreg !== '8000.0' ||
      proposal.faucet_balance_after_tcreg !== '100000.0' ||
      proposal.gas_budget_eth !== '0.001' ||
      proposal.gas_limit_ceiling !== '100000' ||
      proposal.transaction.chainId !== 11155111 ||
      proposal.transaction.from.toLowerCase() !== proposal.treasury.toLowerCase() ||
      proposal.transaction.to.toLowerCase() !== tokenAddress.toLowerCase() ||
      proposal.transaction.data.toLowerCase() !== expectedData.toLowerCase() ||
      proposal.transaction.value_wei !== '0' ||
      proposal.approved_policy_candidate?.drip_tcreg !== '100' ||
      proposal.approved_policy_candidate?.cumulative_distribution_cap_tcreg !== '1000000' ||
      proposal.approved_policy_candidate?.daily_distribution_cap_tcreg !== '50000' ||
      proposal.approved_policy_candidate?.address_lifetime_cap_tcreg !== '100' ||
      proposal.approved_policy_candidate?.hot_wallet_float_tcreg !== '100000' ||
      proposal.approved_policy_candidate?.distribution_open !== false) {
    throw new Error('Proposal differs from the exact reviewed R28 tranche and closed-claims policy');
  }
  const [statsResponse, supplyResponse, healthResponse] = await Promise.all([
    fetch('https://faucet.testnet.cregnet.dev/api/stats', { signal: AbortSignal.timeout(20000) }),
    fetch('https://faucet.testnet.cregnet.dev/api/supply', { signal: AbortSignal.timeout(20000) }),
    fetch('https://faucet.testnet.cregnet.dev/health', { signal: AbortSignal.timeout(20000) }),
  ]);
  if (!statsResponse.ok || !supplyResponse.ok || !healthResponse.ok) {
    throw new Error('Public faucet preflight endpoint is unavailable');
  }
  const [stats, publicSupply, health] = await Promise.all([
    statsResponse.json(), supplyResponse.json(), healthResponse.json(),
  ]);
  if (stats.status !== 'available' || stats.ledger_reconciled !== true ||
      stats.token_claims_enabled !== false || stats.token_distribution_open !== false ||
      stats.token_claim_policy_ready !== true ||
      stats.stats?.unresolved_claims !== 0 ||
      stats.faucet_address.toLowerCase() !== recipient.toLowerCase() ||
      stats.token_contract.toLowerCase() !== tokenAddress.toLowerCase() ||
      BigInt(stats.faucet_balance) !== initialBalance ||
      BigInt(stats.drip_amount) !== parseUnits('100', 18) ||
      BigInt(stats.token_allocation_wei) !== parseUnits('1000000', 18) ||
      BigInt(stats.token_daily_cap_wei) !== parseUnits('50000', 18) ||
      BigInt(stats.token_address_lifetime_cap_wei) !== parseUnits('100', 18) ||
      BigInt(stats.token_hot_wallet_float_wei) !== finalBalance ||
      publicSupply.status !== 'available' ||
      publicSupply.chain_id !== 11155111 ||
      publicSupply.faucet.address.toLowerCase() !== recipient.toLowerCase() ||
      BigInt(publicSupply.faucet.balance_wei) !== initialBalance ||
      BigInt(publicSupply.total_supply_wei) !== parseUnits('20000000', 18) ||
      health.ledger_reconciled !== true) {
    throw new Error('Public faucet ledger, distribution gate, or policy differs from reviewed state');
  }
  const provider = new JsonRpcProvider(proposal.rpc_url);
  if ((await provider.getNetwork()).chainId !== 11155111n) throw new Error('Not Sepolia');
  const secret = spawnSync(process.platform === 'win32' ? 'gcloud.cmd' : 'gcloud', [
    'secrets', 'versions', 'access', 'latest', '--secret=' + proposal.signing_secret_name,
    '--project=' + proposal.secret_project, '--quiet',
  ], { encoding: 'utf8', shell: process.platform === 'win32', windowsHide: true });
  if (secret.status !== 0 || !secret.stdout.trim()) throw new Error('Treasury signing secret is unavailable');
  const raw = secret.stdout.trim();
  const key = raw.startsWith('{') ? JSON.parse(raw).private_key : raw;
  const signer = new Wallet(key, provider);
  if (signer.address.toLowerCase() !== proposal.treasury.toLowerCase()) {
    throw new Error('Signing key does not match the reviewed treasury');
  }
  const token = new Contract(tokenAddress, [
    'function owner() view returns(address)', 'function treasury() view returns(address)',
    'function totalSupply() view returns(uint256)', 'function balanceOf(address) view returns(uint256)',
    'function transfer(address,uint256) returns(bool)',
  ], signer);
  const [code, owner, treasury, supply, treasuryBefore, faucetBefore, nonce, fees, treasuryEth] =
    await Promise.all([
      provider.getCode(tokenAddress), token.owner(), token.treasury(), token.totalSupply(),
      token.balanceOf(signer.address), token.balanceOf(recipient),
      provider.getTransactionCount(signer.address, 'pending'), provider.getFeeData(),
      provider.getBalance(signer.address),
    ]);
  if (keccak256(code) !== proposal.token_code_keccak256 ||
      owner.toLowerCase() !== proposal.token_owner.toLowerCase() ||
      treasury.toLowerCase() !== proposal.treasury.toLowerCase() ||
      supply !== parseUnits('20000000', 18)) throw new Error('Token identity or supply changed');
  if (faucetBefore !== initialBalance || treasuryBefore < amount ||
      nonce !== proposal.transaction.pending_nonce_at_preparation) {
    throw new Error('Balance or pending nonce changed; prepare a new proposal');
  }
  const transaction = {
    to: tokenAddress, data: proposal.transaction.data, value: 0n,
    chainId: 11155111, nonce,
  };
  const estimatedGas = await provider.estimateGas({ ...transaction, from: signer.address });
  const gasLimit = (estimatedGas * 125n + 99n) / 100n;
  if (gasLimit > 100_000n) throw new Error('Gas estimate exceeds the approved limit');
  if (!fees.maxFeePerGas || fees.maxPriorityFeePerGas == null ||
      gasLimit * fees.maxFeePerGas > gasBudget || treasuryEth < gasBudget) {
    throw new Error('Gas cost exceeds the reviewed 0.001 Sepolia ETH budget');
  }
  transaction.gasLimit = gasLimit;
  transaction.maxFeePerGas = fees.maxFeePerGas;
  transaction.maxPriorityFeePerGas = fees.maxPriorityFeePerGas;
  const evidence = {
    status: 'ready_to_broadcast', approval: 'Exact 92,000 tCREG treasury tranche; maximum 0.001 Sepolia ETH gas',
    chain_id: 11155111, token_contract: tokenAddress, treasury: signer.address,
    recipient, amount_wei: amount.toString(), nonce, gas_limit: gasLimit.toString(),
    max_fee_per_gas_wei: fees.maxFeePerGas.toString(),
    faucet_balance_before_wei: faucetBefore.toString(),
    treasury_balance_before_wei: treasuryBefore.toString(),
    total_supply_before_wei: supply.toString(),
    token_claims_remain_closed: true,
  };
  const save = () => {
    const temporaryPath = evidencePath + '.tmp';
    fs.writeFileSync(temporaryPath, JSON.stringify(evidence, null, 2) + '\n');
    fs.renameSync(temporaryPath, evidencePath);
  };
  fs.mkdirSync(path.dirname(evidencePath), { recursive: true });
  save();
  const signedTransaction = await signer.signTransaction(transaction);
  evidence.transaction_hash = keccak256(signedTransaction);
  evidence.status = 'signed_submission_uncertain';
  save();
  const sent = await provider.broadcastTransaction(signedTransaction);
  if (sent.hash.toLowerCase() !== evidence.transaction_hash.toLowerCase()) {
    throw new Error('Broadcast hash differed from the saved signed hash');
  }
  evidence.status = 'broadcast_receipt_pending';
  save();
  console.log(JSON.stringify({ status: evidence.status, transaction_hash: evidence.transaction_hash }));
  const receipt = await sent.wait(2, 180000);
  if (!receipt || receipt.status !== 1) throw new Error('No successful receipt; inspect the saved transaction hash');
  Object.assign(evidence, {
    status: 'receipt_confirmed_balances_pending', block_number: receipt.blockNumber,
    confirmations: 2, gas_used: receipt.gasUsed.toString(),
    gas_cost_eth: formatEther(receipt.gasUsed * receipt.gasPrice),
  });
  save();
  const [treasuryAfter, faucetAfter, supplyAfter] = await Promise.all([
    token.balanceOf(signer.address, { blockTag: receipt.blockNumber }),
    token.balanceOf(recipient, { blockTag: receipt.blockNumber }),
    token.totalSupply({ blockTag: receipt.blockNumber }),
  ]);
  if (faucetAfter !== finalBalance || treasuryAfter !== treasuryBefore - amount || supplyAfter !== supply) {
    throw new Error('Post-transfer balances or total supply do not match the approved operation');
  }
  Object.assign(evidence, {
    status: 'confirmed',
    faucet_balance_after_wei: faucetAfter.toString(),
    treasury_balance_after_wei: treasuryAfter.toString(),
    total_supply_after_wei: supplyAfter.toString(),
  });
  save();
  console.log(JSON.stringify({ status: evidence.status, transaction_hash: sent.hash, block_number: receipt.blockNumber }));
}

main().catch(error => { console.error('Funding stopped:', error.shortMessage || error.message); process.exitCode = 1; });
