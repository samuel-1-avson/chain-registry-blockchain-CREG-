// Read-only Sepolia proposal. This script has no signer and cannot broadcast.
const fs = require('node:fs');
const path = require('node:path');
const { JsonRpcProvider, Contract, parseUnits, formatUnits, formatEther, keccak256 } = require('../../scripts/evm-tool/node_modules/ethers');

async function main() {
  const output = process.argv[2];
  if (!output) throw new Error('Provide the evidence JSON path');
  const manifest = JSON.parse(fs.readFileSync(path.join(__dirname, 'deployment-manifest.json'), 'utf8'));
  const provider = new JsonRpcProvider('https://ethereum-sepolia-rpc.publicnode.com');
  const network = await provider.getNetwork();
  if (network.chainId !== 11155111n) throw new Error('Sepolia chain ID mismatch');
  const recipient = '0x721C8dBD8208CA9dBdcC12AE9339225458608B81';
  const token = new Contract(manifest.cregToken, [
    'function treasury() view returns(address)',
    'function owner() view returns(address)',
    'function decimals() view returns(uint8)',
    'function balanceOf(address) view returns(uint256)',
    'function transfer(address,uint256) returns(bool)',
  ], provider);
  const amount = parseUnits('10000', 18);
  const [code, treasury, owner, decimals, reserve, faucetBalance, ethBalance, nonce, fees] = await Promise.all([
    provider.getCode(manifest.cregToken), token.treasury(), token.owner(), token.decimals(),
    token.balanceOf(manifest.deployer), token.balanceOf(recipient), provider.getBalance(manifest.deployer),
    provider.getTransactionCount(manifest.deployer, 'pending'), provider.getFeeData(),
  ]);
  if (code === '0x' || decimals !== 18n || treasury.toLowerCase() !== manifest.deployer.toLowerCase()) throw new Error('Token/treasury verification failed');
  if (owner.toLowerCase() !== manifest.governance.toLowerCase()) throw new Error('Token governance owner mismatch');
  if (reserve < amount) throw new Error('Insufficient treasury tokens');
  const call = { from: treasury, to: manifest.cregToken, data: token.interface.encodeFunctionData('transfer', [recipient, amount]), value: 0n };
  const [result, estimatedGas] = await Promise.all([
    provider.call(call), provider.estimateGas(call),
  ]);
  if (!token.interface.decodeFunctionResult('transfer', result)[0]) throw new Error('Read-only transfer call returned false');
  const gasLimit = (estimatedGas * 125n + 99n) / 100n;
  const gasBudget = parseUnits('0.001', 18);
  const maximumFeePerGas = gasBudget / gasLimit;
  if (!fees.maxFeePerGas || fees.maxFeePerGas > maximumFeePerGas || ethBalance < gasBudget) throw new Error('Funding cannot fit the bounded gas policy');
  const proposal = {
    timestamp_utc: new Date().toISOString(), status: 'prepared_not_signed_or_broadcast',
    chain_id: Number(network.chainId), rpc: provider._getConnection().url,
    token: manifest.cregToken, token_code_keccak256: keccak256(code), token_owner: owner,
    treasury, recipient, token_amount: '10000', token_amount_atomic: amount.toString(),
    standard_drips: 10, treasury_tokens_before: formatUnits(reserve, 18), faucet_tokens_before: formatUnits(faucetBalance, 18),
    treasury_eth_before: formatEther(ethBalance), gas_budget_eth: '0.001',
    estimated_gas: estimatedGas.toString(), gas_limit: gasLimit.toString(),
    max_fee_per_gas_policy_wei: maximumFeePerGas.toString(),
    current_suggested_max_fee_per_gas_wei: fees.maxFeePerGas.toString(),
    transaction: { chainId: 11155111, from: treasury, to: call.to, data: call.data, value: '0', nonce },
    read_only_eth_call: 'transfer returns true', requires_specific_funding_approval: true,
    signing_secret_name: 'creg-phase-c-treasury-v2-evm', secret_project: 'gen-lang-client-0022105784',
    recheck_before_execution: ['chain ID', 'derived signer equals treasury', 'token code and owner', 'pending nonce', 'balances', 'gas budget', 'recipient'],
  };
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, JSON.stringify(proposal, null, 2) + '\n');
  console.log(JSON.stringify(proposal, null, 2));
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
