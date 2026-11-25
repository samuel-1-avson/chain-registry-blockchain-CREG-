// Run with: npm run test:unit (Node's built-in test runner, no extra deps).
import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  blockTxCount,
  findingCategoryLabel,
  formatViolationTitle,
  normalizeBlock,
  normalizeBlocks,
  normalizeTransaction,
  packageStatusVariant,
  txDisplayStatus,
} from './normalize.js'

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

// eco-adv-12: shape of `Transaction::RevokeV2` as serialized by the node.
const revokeV2 = {
  type: 'revoke_v2',
  request: {
    chain_id: 'creg-testnet',
    genesis_hash: 'ab'.repeat(32),
    package_canonical: 'npm:left-padx@1.4.2',
    reason: 'credential exfiltration',
    revoker_pubkey: 'cd'.repeat(32),
    signature: 'ef'.repeat(64),
    signature_version: 1,
  },
  evidence_hash: '12'.repeat(32),
}

test('eco-adv-12: a finalized block containing revoke_v2 shows a red revocation, never finalized', () => {
  const block = normalizeBlock({ ...liveBlock, transactions: [revokeV2] })
  const [tx] = block.transactions
  assert.equal(tx.canonical, 'npm:left-padx@1.4.2')
  assert.equal(tx.publisher, 'cd'.repeat(32))
  assert.equal(tx.reason, 'credential exfiltration')
  assert.equal(tx.status, 'revoked')
  assert.deepEqual(txDisplayStatus(tx, { finalized: true, included: true }), {
    label: 'revoked',
    variant: 'error',
  })
})

test('legacy revoke is still a revocation', () => {
  const tx = normalizeTransaction({ type: 'revoke', package_canonical: 'npm:x@1', reason: 'r', revoked_by: 'k', evidence_hash: 'e' })
  assert.equal(tx.status, 'revoked')
  assert.equal(txDisplayStatus(tx, { finalized: true }).variant, 'error')
})

test('quarantined publications are amber and unknown types are neutral', () => {
  const held = normalizeTransaction({
    type: 'publish_authorized_v1',
    record: { id: { ecosystem: 'npm', name: 'held', version: '1.0.0' }, status: { Quarantined: { reason: 'r', expires_at_unix: 1 } } },
  })
  assert.equal(held.package_status, 'quarantined')
  assert.deepEqual(txDisplayStatus(held, { finalized: true }), { label: 'quarantined', variant: 'warning' })

  assert.deepEqual(txDisplayStatus({ type: 'quarantine_v1' }, { finalized: true }), { label: 'quarantined', variant: 'warning' })
  assert.deepEqual(txDisplayStatus({ type: 'quarantine_resolution_v1', resolution: 'released' }, { finalized: true }), { label: 'released', variant: 'success' })
  assert.deepEqual(txDisplayStatus({ type: 'quarantine_resolution_v1', resolution: 'rejected' }, { finalized: true }), { label: 'rejected', variant: 'error' })
  assert.deepEqual(txDisplayStatus({ type: 'quarantine_expiry' }, { finalized: true }), { label: 'revoked', variant: 'error' })
  assert.deepEqual(txDisplayStatus({ type: 'future_unknown_tx_v9' }, { finalized: true }), { label: 'UNKNOWN', variant: 'muted' })
  assert.deepEqual(txDisplayStatus({}, { finalized: true }), { label: 'UNKNOWN', variant: 'muted' })

  const published = normalizeBlock(liveBlock).transactions[0]
  assert.deepEqual(txDisplayStatus(published, { finalized: true }), { label: 'finalized', variant: 'success' })
  assert.deepEqual(txDisplayStatus(published, { included: true }), { label: 'included', variant: 'info' })
})

test('package status variants never default to success', () => {
  assert.equal(packageStatusVariant('verified'), 'success')
  assert.equal(packageStatusVariant('Quarantined'), 'warning')
  assert.equal(packageStatusVariant('revoked'), 'error')
  assert.equal(packageStatusVariant('pending'), 'info')
  for (const other of ['', 'neutral', 'quarantined_v2', undefined, null, 7, '__proto__', 'constructor']) {
    assert.equal(packageStatusVariant(other), 'muted', String(other))
  }
})

test('closed finding category and kernel violation dictionaries reject arbitrary text', () => {
  assert.equal(findingCategoryLabel('credential_exfiltration'), 'Credential exfiltration')
  assert.equal(findingCategoryLabel('<script>alert(1)</script>'), 'Unknown category')
  assert.equal(formatViolationTitle({ code: 'KB-CANARY-EXFIL' }), 'Canary secret exfiltration')
  assert.equal(
    formatViolationTitle({ code: null, title: 'spoofed', unknown: 'UNKNOWN(deadbeef)' }),
    'Unknown violation code (UNKNOWN(deadbeef))',
  )
  assert.equal(formatViolationTitle({ code: 'INJECTED', title: '<img src=x>' }), 'Unknown violation code')
})
