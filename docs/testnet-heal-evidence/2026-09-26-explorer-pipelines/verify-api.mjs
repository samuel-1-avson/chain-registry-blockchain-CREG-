import assert from 'node:assert/strict'
import fs from 'node:fs/promises'

const [output, base = 'http://127.0.0.1:18080'] = process.argv.slice(2)
assert.ok(output)
const records = []
const read = async endpoint => {
  const response = await fetch(base + endpoint, { signal: AbortSignal.timeout(15000) })
  const payload = await response.json()
  records.push({ endpoint, status: response.status, payload })
  assert.equal(response.status, 200, endpoint)
  return payload
}
try {
  const stats = await read('/v1/public/chain/stats')
  assert.equal(stats.tip_height, 4)
  assert.equal(stats.finalized_height, 4)
  assert.equal(stats.finalization_source, 'creg-canonical-store')
  assert.equal(stats.genesis_hash, '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4')
  const all = await read('/v1/public/blocks?limit=5')
  assert.deepEqual(all.blocks.map(block => block.header.height), [4, 3, 2, 1, 0])
  for (const cursor of ['before_height=0', 'after_height=4', 'offset=5']) {
    assert.equal((await read(`/v1/public/blocks?limit=5&${cursor}`)).blocks.length, 0)
  }
  assert.deepEqual((await read('/v1/public/blocks?before_height=3&limit=5')).blocks.map(b => b.header.height), [2, 1, 0])
  const search = await read('/v1/public/search?q=is-number')
  assert.ok(search.matches.some(item => item.title === 'npm:is-number@7.0.0'))
  const anchors = await read('/v1/public/bridge/anchors')
  assert.equal(anchors.anchors.length, 0)
  assert.ok(anchors.note.includes('persisted'))
  const spec = await (await fetch('https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json')).json()
  for (const validator of spec.validator_set.validators) {
    const profile = await read(`/v1/public/validators/${validator.eth_address}`)
    assert.equal(profile.node_id, validator.id)
    assert.equal(Number(profile.stake), 100)
    assert.equal(profile.stake_unit, 'CREG')
    assert.equal(profile.scanned_blocks, 5)
    const expectedProposals = all.blocks.filter(b => b.header.proposer_id === validator.id).length
    assert.equal(profile.recent_proposals.length, expectedProposals)
    const address = await read(`/v1/public/addresses/${validator.eth_address}`)
    assert.equal(address.blocks_proposed, expectedProposals)
    assert.equal(address.is_validator, true)
    const activity = await read(`/v1/public/addresses/${validator.eth_address}/transactions?limit=200&scan=500`)
    assert.equal(activity.transactions.filter(tx => tx.kind === 'propose').length, expectedProposals)
  }
  const publication = await read('/v1/public/blocks/1')
  const publisher = publication.transactions[0].record.publisher_pubkey
  const publisherStats = await read(`/v1/public/publishers/${publisher}`)
  assert.equal(publisherStats.total_packages, 1)
  assert.equal(publisherStats.verified_count, 0)
  assert.equal(publisherStats.revoked_count, 1)
  await fs.writeFile(output, JSON.stringify({ capturedAt: new Date().toISOString(), base, verified: true,
    dataSource: 'Live real chain; read-only checks; no fixture responses', records }, null, 2) + '\n', { flag: 'wx' })
  console.log(JSON.stringify({ verified: true, realRequests: records.length, height: 4, validators: 4 }))
} catch (error) {
  await fs.writeFile(output + '.failure.json', JSON.stringify({ error: error.message, records }, null, 2) + '\n')
  throw error
}
