// Run with: npm run test:unit (Node's built-in test runner, no extra deps).
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { blockTxCount, normalizeBlock, normalizeBlocks } from './normalize.js'

// Shape returned by GET /v1/public/blocks/11 on the R7 testnet (trimmed).
const liveBlock = {
  header: { height: 11, proposer_id: 'validator-1', timestamp: '2026-09-26T22:39:03Z' },
  transactions: [{
    type: 'publish_authorized_v1',
    record: { id: { ecosystem: 'npm', name: 'ms', version: '2.1.3' } },
  }],
  pbft_signatures: [],
  hash: 'a87ff9',
  finalized: true,
}

test('tx_count is derived from returned transactions when the API omits it', () => {
  assert.equal(normalizeBlock(liveBlock).tx_count, 1)
})

test('block list payloads get a per-block tx_count', () => {
  const payload = normalizeBlocks({ blocks: [liveBlock, { ...liveBlock, transactions: [] }] })
  assert.deepEqual(payload.blocks.map((block) => block.tx_count), [1, 0])
})

test('an explicit tx_count is used only when transactions are absent', () => {
  assert.equal(blockTxCount({ tx_count: 3 }), 3)
  assert.equal(blockTxCount({ tx_count: 3, transactions: [{}] }), 1)
  assert.equal(blockTxCount({}), 0)
  assert.equal(blockTxCount({ tx_count: '3' }), 0)
})
