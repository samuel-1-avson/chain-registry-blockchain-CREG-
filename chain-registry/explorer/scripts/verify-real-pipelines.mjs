import { chromium } from '@playwright/test'
import fs from 'node:fs/promises'
import assert from 'node:assert/strict'
import { normalizeStats, normalizeBlock, normalizeBlocks } from '../src/api/normalize.js'

const evidence = process.argv[2]
const base = process.argv[3] || 'http://127.0.0.1:3007'
const expectedHeight = Number(process.argv[4] || 4)
if (!evidence) throw new Error('Usage: node scripts/verify-real-pipelines.mjs EVIDENCE_DIRECTORY [URL] [HEIGHT]')
await fs.mkdir(evidence, { recursive: true })
const read = async endpoint => {
  const response = await fetch(base + endpoint)
  assert.equal(response.status, 200, endpoint)
  return response.json()
}
const stats = normalizeStats(await read('/v1/public/chain/stats'))
assert.equal(stats.current_height, expectedHeight)
const consensus = await read('/v1/validator/consensus/state')
const block = normalizeBlock(await read(`/v1/public/blocks/${expectedHeight}`))
assert.equal(block.height, expectedHeight)
assert.ok(block.producer)
const blocks = normalizeBlocks(await read('/v1/public/blocks?limit=5'))
if (expectedHeight > 0) assert.equal(blocks.blocks[0].height, expectedHeight)
if (expectedHeight === 4) {
  const publication = normalizeBlock(await read('/v1/public/blocks/1'))
  assert.equal(publication.transactions[0].canonical, 'npm:is-number@7.0.0')
  assert.equal(publication.transactions[0].version, '7.0.0')
  assert.equal(publication.transactions[0].publisher.length, 64)
  assert.equal(block.transactions[0].status, 'revoked')
  assert.ok(block.votes.length >= 3)
}
const browser = await chromium.launch({ headless: true })
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } })
await context.addInitScript(() => {
  window.__eventSubscriptions = []
  const original = EventSource.prototype.addEventListener
  EventSource.prototype.addEventListener = function (type, ...args) {
    window.__eventSubscriptions.push(type)
    return original.call(this, type, ...args)
  }
})
const page = await context.newPage()
const errors = []
page.on('pageerror', error => errors.push(error.message))
const routes = ['/', '/blocks', `/block/${expectedHeight}`, '/validators', '/validator/validator-1',
  '/consensus', '/network', '/pending', '/metrics', '/bridge', '/events', '/richlist', '/reorgs', '/proof']
if (expectedHeight === 4) routes.push('/block/1', '/package/npm%3Ais-number%407.0.0', '/tx/npm%3Ais-number%407.0.0')
const records = []
try {
  for (const route of routes) {
    await page.goto(base + route, { waitUntil: 'domcontentloaded' })
    await page.waitForTimeout(1800)
    const main = await page.locator('main').innerText()
    assert.ok(main.length > 20, route + ': blank page')
    if (route === `/block/${expectedHeight}`) {
      assert.ok(main.includes(`Block #${expectedHeight}`))
      assert.ok(main.includes(block.producer))
    }
    if (route === '/validators') assert.ok(main.includes('validator-1'))
    if (route === '/validator/validator-1') assert.ok(!main.includes('Invalid address'), main)
    if (route === '/network') {
      for (const validator of consensus.validators || []) {
        assert.ok(main.includes(validator.alias || validator.id), main)
      }
      const displayedQuorum = main.match(/PBFT quorum\s+(\d+)/i)
      assert.ok(displayedQuorum, main)
      assert.equal(Number(displayedQuorum[1]), consensus.quorum)
    }
    if (route === '/pending' && stats.pending_tx_count === 0) {
      assert.ok(main.includes('Pending pool is empty'), main)
    }
    if (route === '/tx/npm%3Ais-number%407.0.0') assert.ok(main.toLowerCase().includes('revoked'), main)
    records.push({ route, main, subscriptions: await page.evaluate(() => window.__eventSubscriptions) })
    await page.screenshot({ path: `${evidence}/${route === '/' ? 'dashboard' : route.replaceAll('/', '_')}.png`, fullPage: true })
  }
  assert.equal(errors.length, 0, JSON.stringify(errors))
  assert.ok(records[0].subscriptions.includes('block_produced'))
  const report = { capturedAt: new Date().toISOString(), base, stats, verified: true,
    dataSource: 'live real node, no intercepted or fixture responses', records, errors }
  await fs.writeFile(`${evidence}/browser-verification.json`, JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify({ verified: true, chain: stats.chain_id, height: expectedHeight, routes: records.length, errors }))
} catch (error) {
  await fs.writeFile(`${evidence}/browser-failure.json`, JSON.stringify({ error: error.message, records, errors }, null, 2))
  throw error
} finally { await browser.close() }
