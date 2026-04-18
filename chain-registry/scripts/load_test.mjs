/**
 * scripts/load_test.mjs
 * 
 * Load tester to run against the chain-registry-node locally.
 * Simulates high-throughput package registrations to hit 1M txs / 10K blocks capacity.
 */
import { performance } from 'node:perf_hooks';

const NODE_URL = process.env.CREG_URL || 'http://127.0.0.1:8080';
const TARGET_TXS = 10000; // Scaled down for quick local dev tests, scale up for actual CI test

async function sleep(ms) {
  return new Promise(r => setTimeout(r, ms));
}

async function stress() {
  console.log(`Starting load test against ${NODE_URL}`);
  let success = 0;
  let errors = 0;
  const start = performance.now();

  for (let i = 0; i < TARGET_TXS; i++) {
    // Generate a payload imitating PublishRequest
    const payload = {
      id: "npm:" + Math.random().toString(36).substring(2) + "@1.0.0",
      content_hash: "0000000000000000000000000000000000000000000000000000000000000000",
      ipfs_cid: "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi",
      publisher_pubkey: "loadtest",
      signature: "0000"
    };

    try {
      const res = await fetch(`${NODE_URL}/v1/packages`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      });
      // In a real test, the node will likely reject standard signature because it's mock,
      // but the API latency parsing the signature is what we measure for backpressure.
      if (res.status === 200 || res.status === 400 || res.status === 401) {
        success++;
      }
    } catch (e) {
      errors++;
    }

    if (i % 500 === 0 && i !== 0) {
      console.log(`[Iter ${i}] Success: ${success}, Errors: ${errors}`);
    }
  }

  const end = performance.now();
  const timeSecs = (end - start) / 1000;
  console.log('\n--- Load Test Results ---');
  console.log(`Total Tx: ${TARGET_TXS}`);
  console.log(`Time: ${timeSecs.toFixed(2)}s`);
  console.log(`RPS (Requests Per Second): ${(TARGET_TXS / timeSecs).toFixed(2)}`);
  console.log(`Success: ${success}`);
  console.log(`Network Errors: ${errors}`);
}

stress().catch(console.error);
