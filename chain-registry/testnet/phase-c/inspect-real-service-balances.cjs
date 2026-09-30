// Read actual Sepolia balances without printing or persisting service keys.
const fs = require('node:fs');
const { spawnSync } = require('node:child_process');
const { ethers } = require('../../scripts/evm-tool/node_modules/ethers');
const deployment = require('./deployment-manifest.json');
const outputPath = process.argv[2];
if (!outputPath || fs.existsSync(outputPath)) throw new Error('A fresh evidence path is required');
const provider = new ethers.JsonRpcProvider('https://ethereum-sepolia-rpc.publicnode.com');
const token = new ethers.Contract(deployment.cregToken, ['function balanceOf(address) view returns (uint256)'], provider);

async function main() {
  if ((await provider.getNetwork()).chainId !== 11155111n) throw new Error('Expected real Sepolia');
  const wallets = [];
  for (const role of ['faucet', 'relayer']) {
    const secret = `creg-fresh-testnet-${role}-evm`;
    const result = spawnSync('gcloud.cmd', ['secrets', 'versions', 'access', 'latest', `--secret=${secret}`, '--project=gen-lang-client-0022105784'], { encoding: 'utf8', shell: true, windowsHide: true });
    if (result.status !== 0 || !/^0x[0-9a-fA-F]{64}$/.test(result.stdout.trim())) throw new Error(`Could not securely load the existing ${role} key`);
    let wallet;
    try { wallet = new ethers.Wallet(result.stdout.trim()); }
    catch { throw new Error(`Invalid existing ${role} key`); }
    result.stdout = '';
    wallets.push({ role, secret_id: secret, address: wallet.address });
    wallet = null;
  }
  wallets.push({ role: 'treasury', address: deployment.deployer });
  for (const wallet of wallets) {
    const [native, tokens] = await Promise.all([provider.getBalance(wallet.address), token.balanceOf(wallet.address)]);
    wallet.native_wei = native.toString();
    wallet.token_wei = tokens.toString();
  }
  const evidence = { captured_at_utc: new Date().toISOString(), network: 'real Ethereum Sepolia', chain_id: 11155111, token_contract: deployment.cregToken, wallets, secret_values_included: false, transactions_broadcast: false, public_cutover: false };
  fs.writeFileSync(outputPath, JSON.stringify(evidence, null, 2) + '\n');
  console.log(JSON.stringify(evidence));
}
main().catch(error => { console.error(error.message); process.exitCode = 1; }).finally(() => provider.destroy());
