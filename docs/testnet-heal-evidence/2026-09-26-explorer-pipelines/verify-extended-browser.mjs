import { chromium } from '../../../chain-registry/explorer/node_modules/@playwright/test/index.mjs'
import fs from 'node:fs/promises'
import assert from 'node:assert/strict'

const [directory, base = 'http://127.0.0.1:3007', mode = 'capture'] = process.argv.slice(2)
assert.ok(directory)
await fs.mkdir(directory, { recursive: true })
const read = async endpoint => {
  const response = await fetch(base + endpoint, { signal: AbortSignal.timeout(15000) })
  assert.equal(response.status, 200, endpoint)
  return response.json()
}
const stats = await read('/v1/public/chain/stats')
const specUrl = stats.tip_height === 4 ? 'https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json'
  : 'https://spec.testnet.cregnet.dev/chain-spec.json'
const spec = await (await fetch(specUrl)).json()
const address = spec.validator_set.validators[0].eth_address
const routes = ['/packages', `/address/${address}`, `/validator/${address}`, '/search?q=is-number',
  '/governance', '/wallet', '/publisher', '/about']
if (stats.tip_height === 4) {
  const publication = await read('/v1/public/blocks/1')
  const publisher = publication.transactions[0].record.publisher_pubkey
  routes.push(`/publisher/${publisher}`)
}
const browser = await chromium.launch({ headless: true })
const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } })
const errors = [], failedRequests = [], records = []
page.on('pageerror', error => errors.push(error.message))
page.on('requestfailed', request => failedRequests.push({ url: request.url(), error: request.failure()?.errorText }))
try {
  for (const route of routes) {
    await page.goto(base + route, { waitUntil: 'domcontentloaded' })
    await page.waitForTimeout(2400)
    const main = await page.locator('main').innerText()
    assert.ok(main.length > 20, route)
    if (mode === 'fixed' && route.startsWith('/address/') && stats.tip_height === 4) {
      assert.ok(main.includes('100 CREG'), main)
      assert.ok(!main.includes('0.000000'), main)
      assert.ok(main.toLowerCase().includes('propose'), main)
    }
    if (mode === 'fixed' && route.startsWith('/validator/')) {
      assert.ok(!main.includes('Estimated uptime'), main)
      assert.ok(main.includes('Uptime measurements are unavailable'), main)
    }
    if (mode === 'fixed' && route.startsWith('/publisher/')) {
      assert.ok(main.includes('npm:is-number@7.0.0'), main)
      assert.ok(main.toLowerCase().includes('revoked'), main)
    }
    const l1Links = await page.locator('main a[href*="etherscan.io"]').evaluateAll(items => items.map(item => item.href))
    if (mode === 'fixed') assert.ok(l1Links.every(link => link.startsWith('https://sepolia.etherscan.io/')), JSON.stringify(l1Links))
    records.push({ route, main, l1Links })
    await page.screenshot({ path: `${directory}/${records.length}.png`, fullPage: true })
  }
  assert.equal(errors.length, 0, JSON.stringify(errors))
  await fs.writeFile(`${directory}/browser-verification.json`, JSON.stringify({ capturedAt: new Date().toISOString(),
    base, chainId: stats.chain_id, height: stats.tip_height, mode, verified: true,
    dataSource: 'Live real HTTP responses; no intercepts or fixtures', records, errors, failedRequests }, null, 2) + '\n')
  console.log(JSON.stringify({ verified: true, routes: records.length, mode, errors, failedRequests }))
} catch (error) {
  await fs.writeFile(`${directory}/browser-failure.json`, JSON.stringify({ error: error.message, records, errors, failedRequests }, null, 2))
  throw error
} finally { await browser.close() }
