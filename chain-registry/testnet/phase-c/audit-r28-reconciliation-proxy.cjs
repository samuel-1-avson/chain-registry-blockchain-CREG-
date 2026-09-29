// Read-only check that the live Sepolia proxy supports all current history reads.
const { JsonRpcProvider, Contract, keccak256, parseUnits } = require('../../scripts/evm-tool/node_modules/ethers');

async function main() {
  const provider = new JsonRpcProvider('https://api.testnet.cregnet.dev/sepolia');
  const tokenAddress = '0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6';
  const faucet = '0x721C8dBD8208CA9dBdcC12AE9339225458608B81';
  const fundingHash = '0x11f797c1d4514973ee88770f359ce63dcafa22c27de8996c909f12d9f713df21';
  const safe = await provider.getBlock('finalized');
  if ((await provider.getNetwork()).chainId !== 11155111n || safe.number < 11810180) {
    throw new Error('Sepolia proxy has not finalized the funding transfer');
  }
  const [codeAtSafe, logs] = await Promise.all([
    provider.getCode(tokenAddress, 'latest'),
    provider.getLogs({
      address: tokenAddress, fromBlock: 11809706, toBlock: safe.number,
      topics: [keccak256(Buffer.from('Transfer(address,address,uint256)'))],
    }),
  ]);
  if (codeAtSafe === '0x' ||
      keccak256(codeAtSafe) !== '0xee4ee60af45e022c2673e37e21781b5d88d97164e768ad1d65293122aa1d2b16') {
    throw new Error('Live token code hash differs');
  }
  const token = new Contract(tokenAddress, [
    'function balanceOf(address) view returns(uint256)',
    'function totalSupply() view returns(uint256)',
  ], provider);
  const [balance, supply] = await Promise.all([
    token.balanceOf(faucet, { blockTag: safe.number }),
    token.totalSupply({ blockTag: safe.number }),
  ]);
  if (balance !== parseUnits('100000', 18) || supply !== parseUnits('20000000', 18)) {
    throw new Error('Exact-block token state differs from the confirmed funding receipt');
  }
  const matching = logs.filter(log => log.transactionHash.toLowerCase() === fundingHash.toLowerCase());
  if (matching.length !== 1) throw new Error('Expected one canonical funding Transfer log');
  console.log(JSON.stringify({
    status: 'read_only_history_source_ready', finalized_block: safe.number,
    funding_log_count: matching.length, faucet_balance_wei: balance.toString(),
    total_supply_wei: supply.toString(), requests_mocked: false,
  }));
}

main().catch(error => {
  console.error(JSON.stringify({ error: error.message, code: error.code,
    rpc_error: error.info?.error?.message }));
  process.exitCode = 1;
});
