// Create a dedicated real Sepolia recipient whose key stays in GCP Secret Manager.
const { spawnSync } = require('node:child_process');
const { Wallet } = require('../../scripts/evm-tool/node_modules/ethers');

const name = 'creg-phase-c-r29-acceptance-evm';
const project = 'gen-lang-client-0022105784';
const binary = process.platform === 'win32' ? 'gcloud.cmd' : 'gcloud';
const options = { encoding: 'utf8', shell: process.platform === 'win32', windowsHide: true };

const existing = spawnSync(binary, ['secrets', 'describe', name,
  '--project=' + project, '--format=value(name)', '--quiet'], options);
if (existing.status === 0) throw new Error('Acceptance secret already exists; inspect it instead of replacing it');
const wallet = Wallet.createRandom();
const created = spawnSync(binary, ['secrets', 'create', name,
  '--replication-policy=automatic', '--data-file=-', '--project=' + project, '--quiet'],
  { ...options, input: wallet.privateKey });
if (created.status !== 0) throw new Error('Could not create protected acceptance wallet secret');
const stored = spawnSync(binary, ['secrets', 'versions', 'access', 'latest',
  '--secret=' + name, '--project=' + project, '--quiet'], options);
if (stored.status !== 0 || new Wallet(stored.stdout.trim()).address !== wallet.address) {
  throw new Error('Protected acceptance wallet verification failed');
}
console.log(JSON.stringify({ secret_name: name, address: wallet.address, key_location: 'GCP Secret Manager' }));
