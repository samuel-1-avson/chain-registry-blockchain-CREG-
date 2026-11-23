// Display projections of real responses; signed payloads remain in header/record.
export function normalizeStats(stats) {
  // Older nodes put L1 checkpoint height in the CREG finality fields.
  return { ...stats, current_height: stats.tip_height ?? stats.current_height,
    finalized_height: stats.finalization_source ? stats.finalized_height : null,
    finalization_lag: stats.finalization_source ? stats.finalization_lag : null }
}

// ECA-34 (ADR-019 Increment 1): one mapping from chain transaction types and
// package statuses to display state. `revoke_v2`, which is what the current
// node emits for every revocation, used to fall through to a green
// "finalized" badge, and a quarantined package rendered as success. Unknown
// values now render neutral, never as success.

/** Serde `type` tags of `common::Transaction` (snake_case). */
export const TX_TYPES = Object.freeze([
  'publish',
  'publish_authorized_v1',
  'revoke',
  'revoke_v2',
  'slash',
  'validator_join',
  'validator_leave',
  'rotate_publisher_key',
  'namespace_lifecycle',
  'namespace_lifecycle_v2',
])

export const REVOCATION_TX_TYPES = Object.freeze(['revoke', 'revoke_v2'])

export function isRevocationTx(tx) {
  return REVOCATION_TX_TYPES.includes(tx?.type)
}

const PACKAGE_STATUS_VARIANTS = Object.freeze({
  verified: 'success',
  pending: 'info',
  quarantined: 'warning',
  revoked: 'error',
  rejected: 'error',
})

/** Badge variant for a package status string; unknown → 'muted'. */
export function packageStatusVariant(status) {
  const key = typeof status === 'string' ? status.toLowerCase() : ''
  return Object.prototype.hasOwnProperty.call(PACKAGE_STATUS_VARIANTS, key)
    ? PACKAGE_STATUS_VARIANTS[key]
    : 'muted'
}

/**
 * Display status of one transaction as `{ label, variant }`.
 * `finalized`: the containing block is final. `included`: it is in a block.
 */
export function txDisplayStatus(tx, { finalized = false, included = false } = {}) {
  if (isRevocationTx(tx)) return { label: 'revoked', variant: 'error' }
  if (!TX_TYPES.includes(tx?.type)) return { label: 'unknown', variant: 'muted' }
  if (tx?.package_status === 'quarantined') return { label: 'quarantined', variant: 'warning' }
  if (tx?.package_status === 'revoked') return { label: 'revoked', variant: 'error' }
  if (finalized) return { label: 'finalized', variant: 'success' }
  if (included) return { label: 'included', variant: 'info' }
  return { label: 'pending', variant: 'info' }
}

export function normalizeTransaction(tx) {
  const record = tx.record || tx
  const id = record.id || {}
  // RevokeV2 carries its target inside the signed request.
  const request = tx.type === 'revoke_v2' && tx.request && typeof tx.request === 'object' ? tx.request : {}
  const canonical = tx.canonical || tx.package_canonical || request.package_canonical
    || (id.ecosystem && id.name && id.version ? `${id.ecosystem}:${id.name}@${id.version}` : undefined)
  const value = record.status
  const packageStatus = typeof value === 'string' ? value.toLowerCase()
    : value && typeof value === 'object' ? Object.keys(value)[0]?.toLowerCase() : undefined
  return { ...tx, canonical, ecosystem: id.ecosystem, name: id.name, version: id.version,
    publisher: tx.publisher || record.publisher_pubkey || tx.revoked_by || request.revoker_pubkey,
    reason: tx.reason ?? request.reason,
    ipfs_cid: record.ipfs_cid, payload_hash: record.content_hash,
    validation: record.deterministic_risk, included_at: record.published_at,
    package_status: packageStatus, status: isRevocationTx(tx) ? 'revoked' : packageStatus }
}

// The block list and block detail endpoints return full blocks without a
// `tx_count` field, so derive it from the transactions actually returned.
export function blockTxCount(block) {
  if (Array.isArray(block.transactions)) return block.transactions.length
  return Number.isSafeInteger(block.tx_count) ? block.tx_count : 0
}

export function normalizeBlock(block) {
  const header = block.header || block
  return { ...block, height: header.height, timestamp: header.timestamp,
    prev_hash: header.prev_hash, merkle_root: header.merkle_root,
    producer: header.proposer_id ?? block.producer,
    votes: block.pbft_signatures ?? block.votes ?? [],
    tx_count: blockTxCount(block),
    transactions: (block.transactions || []).map(normalizeTransaction) }
}

export function normalizeBlocks(payload) {
  return Array.isArray(payload) ? payload.map(normalizeBlock)
    : { ...payload, blocks: (payload.blocks || []).map(normalizeBlock) }
}
