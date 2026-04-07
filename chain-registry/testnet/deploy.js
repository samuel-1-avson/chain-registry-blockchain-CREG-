// Simple contract deployment script for Chain Registry Testnet
const http = require('http');

const RPC_URL = 'http://localhost:8545';
const DEPLOYER_KEY = '0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';

// TestCregToken bytecode (simplified - will be compiled)
const TOKEN_BYTECODE = '0x'; // We'll use eth_sendTransaction to deploy

function sendRpc(method, params = []) {
  return new Promise((resolve, reject) => {
    const data = JSON.stringify({
      jsonrpc: '2.0',
      method: method,
      params: params,
      id: Date.now()
    });

    const options = {
      hostname: 'localhost',
      port: 8545,
      path: '/',
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': data.length
      }
    };

    const req = http.request(options, (res) => {
      let responseData = '';
      res.on('data', (chunk) => responseData += chunk);
      res.on('end', () => {
        try {
          const result = JSON.parse(responseData);
          if (result.error) {
            reject(new Error(result.error.message));
          } else {
            resolve(result.result);
          }
        } catch (e) {
          reject(e);
        }
      });
    });

    req.on('error', reject);
    req.write(data);
    req.end();
  });
}

async function main() {
  console.log('Chain Registry Testnet Deployment');
  console.log('=================================\n');

  try {
    // Get block number to check connection
    console.log('[1/3] Checking Anvil connection...');
    const blockNumber = await sendRpc('eth_blockNumber');
    console.log(`  Connected! Block: ${parseInt(blockNumber, 16)}\n`);

    // Get deployer balance
    console.log('[2/3] Checking deployer balance...');
    const balance = await sendRpc('eth_getBalance', ['0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266', 'latest']);
    console.log(`  Balance: ${parseInt(balance, 16) / 1e18} ETH\n`);

    console.log('[3/3] Ready to deploy!');
    console.log('\nTo deploy contracts, you need Foundry installed.');
    console.log('\nAlternative: Use this command in a separate terminal:');
    console.log('  docker run -it --rm -v "F:/project/chain-registry/chain-registry:/workspace" -w /workspace/contracts/testnet --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest');
    console.log('\nThen inside the container, run:');
    console.log('  forge create TestCregToken.sol:TestCregToken --rpc-url http://creg-testnet-anvil:8545 --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --constructor-args "Test CREG Token" "tCREG"');

  } catch (error) {
    console.error('Error:', error.message);
    console.log('\nMake sure Anvil is running:');
    console.log('  docker-compose -f testnet/docker-compose.testnet.yml up -d anvil');
  }
}

main();
