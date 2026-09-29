// Read-only real Sepolia proposal for the first bounded R28 faucet tranche.
// No signing material is loaded and no transaction is submitted.
const fs = require('node:fs');
const path = require('node:path');
const {
  JsonRpcProvider, Contract, Interface, parseUnits, formatUnits, formatEther, keccak256,
} = require('../../scripts/evm-tool/node_modules/ethers');

async function main() {
  const output = process.argv[2];
  if (!output) throw new Error('Provide an evidence JSON path');
  const manifest = JSON.parse(fs.readFileSync(path.join(__dirname, 'deployment-manifest.json'), 'utf8'));
  const provider = new JsonRpcProvider('https://api.testnet.cregnet.dev/sepolia');
  if ((await provider.getNetwork()).chainId !== 11155111n) throw new Error('Not Sepolia');
  const faucet = '0x721C8dBD8208CA9dBdcC12AE9339225458608B81';
  const iface = new Interface([
    'function balanceOf(address) view returns(uint256)',
    'function totalSupply() view returns(uint256)',
    'function MAX_SUPPLY() view returns(uint256)',
    'function decimals() view returns(uint8)',
    'function owner() view returns(address)',
    'function treasury() view returns(address)',
    'function transfer(address,uint256) returns(bool)',
  ]);
  const token = new Contract(manifest.cregToken, iface, provider);
  const blockNumber = await provider.getBlockNumber();
  const at = { blockTag: blockNumber };
  const [code, treasury, owner, decimals, totalSupply, cap, treasuryBalance, faucetBalance,
    treasuryEth, nonce, fees] = await Promise.all([
    provider.getCode(manifest.cregToken, blockNumber), token.treasury(at), token.owner(at),
    token.decimals(at), token.totalSupply(at), token.MAX_SUPPLY(at),
    token.balanceOf(manifest.deployer, at), token.balanceOf(faucet, at),
    provider.getBalance(manifest.deployer, blockNumber),
    provider.getTransactionCount(manifest.deployer, 'pending'), provider.getFeeData(),
  ]);
  if (code === '0x' || decimals !== 18n) throw new Error('Unexpected token contract or decimals');
  if (treasury.toLowerCase() !== manifest.deployer.toLowerCase()) throw new Error('Treasury changed');
  if (owner.toLowerCase() !== manifest.governance.toLowerCase()) throw new Error('Token owner changed');
  if (totalSupply !== parseUnits('20000000', 18) || cap !== parseUnits('42000000', 18)) {
    throw new Error('Supply or cap changed; review the tokenomics policy');
  }
  const expectedFaucet = parseUnits('8000', 18);
  const workingBalance = parseUnits('100000', 18);
  if (faucetBalance !== expectedFaucet) throw new Error('Faucet balance changed; recompute the tranche');
  const amount = workingBalance - faucetBalance;
  if (amount !== parseUnits('92000', 18) || treasuryBalance < amount) {
    throw new Error('First tranche is no longer 92,000 tCREG from available treasury funds');
  }
  const gasLimitCeiling = 100_000n;
  const gasBudget = parseUnits('0.001', 18);
  const proposal = {
    status: 'prepared_read_only_not_approved_not_signed_not_broadcast',
    prepared_at_utc: new Date().toISOString(),
    chain_id: 11155111,
    snapshot_block: blockNumber,
    rpc_url: 'https://api.testnet.cregnet.dev/sepolia',
    token_contract: manifest.cregToken,
    token_code_keccak256: keccak256(code),
    token_owner: owner,
    treasury,
    faucet,
    total_supply_tcreg: formatUnits(totalSupply, 18),
    cap_tcreg: formatUnits(cap, 18),
    treasury_balance_before_tcreg: formatUnits(treasuryBalance, 18),
    faucet_balance_before_tcreg: formatUnits(faucetBalance, 18),
    faucet_balance_after_tcreg: formatUnits(workingBalance, 18),
    approved_policy_candidate: {
      drip_tcreg: '100', cumulative_distribution_cap_tcreg: '1000000',
      daily_distribution_cap_tcreg: '50000', address_lifetime_cap_tcreg: '100',
      hot_wallet_float_tcreg: '100000', refill_trigger_tcreg: '25000',
      native_eth_drip: '0', distribution_open: false,
    },
    top_up_amount_tcreg: formatUnits(amount, 18),
    top_up_amount_wei: amount.toString(),
    treasury_eth_before: formatEther(treasuryEth),
    gas_limit_ceiling: gasLimitCeiling.toString(),
    gas_budget_eth: '0.001',
    max_fee_per_gas_wei_under_budget: (gasBudget / gasLimitCeiling).toString(),
    suggested_max_fee_per_gas_wei_at_preparation: fees.maxFeePerGas?.toString() ?? null,
    transaction: {
      chainId: 11155111, from: treasury, to: manifest.cregToken,
      data: iface.encodeFunctionData('transfer', [faucet, amount]), value_wei: '0',
      pending_nonce_at_preparation: nonce,
    },
    signing_secret_name: 'creg-phase-c-treasury-v2-evm',
    secret_project: 'gen-lang-client-0022105784',
    recheck_before_execution: [
      'chain ID and token code hash', 'treasury signer address and token ownership',
      'pending nonce and both token balances', 'actual gas estimate and bounded fee',
      'faucet production ledger, current claims, and approved policy status',
    ],
    requests_mocked: false,
    on_chain_transaction_sent: false,
  };
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, JSON.stringify(proposal, null, 2) + '\n');
  console.log(JSON.stringify({
    status: proposal.status, snapshot_block: blockNumber,
    top_up_amount_tcreg: proposal.top_up_amount_tcreg,
    faucet_balance_after_tcreg: proposal.faucet_balance_after_tcreg,
    gas_budget_eth: proposal.gas_budget_eth,
    output,
  }, null, 2));
}

main().catch(error => { console.error(error.shortMessage || error.message); process.exitCode = 1; });
