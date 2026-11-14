// Display projections of real responses; signed payloads remain in header/record.
export function normalizeStats(stats) {
  // Older nodes put L1 checkpoint height in the CREG finality fields.
  return { ...stats, current_height: stats.tip_height ?? stats.current_height,
    finalized_height: stats.finalization_source ? stats.finalized_height : null,
    finalization_lag: stats.finalization_source ? stats.finalization_lag : null }
}

export function normalizeTransaction(tx) {
  const record = tx.record || tx
  const id = record.id || {}
  const canonical = tx.canonical || tx.package_canonical
    || (id.ecosystem && id.name && id.version ? `${id.ecosystem}:${id.name}@${id.version}` : undefined)
  const value = record.status
  const packageStatus = typeof value === 'string' ? value.toLowerCase()
    : value && typeof value === 'object' ? Object.keys(value)[0]?.toLowerCase() : undefined
  return { ...tx, canonical, ecosystem: id.ecosystem, name: id.name, version: id.version,
    publisher: tx.publisher || record.publisher_pubkey || tx.revoked_by,
    ipfs_cid: record.ipfs_cid, payload_hash: record.content_hash,
    validation: record.deterministic_risk, included_at: record.published_at,
    package_status: packageStatus, status: tx.type === 'revoke' ? 'revoked' : packageStatus }
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
