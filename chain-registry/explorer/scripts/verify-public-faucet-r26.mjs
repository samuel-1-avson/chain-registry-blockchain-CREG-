// Read-only browser check of the real public Sepolia faucet during policy pause.
import { chromium } from '@playwright/test'
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'

const base = 'https://faucet.testnet.cregnet.dev'
const evidenceDirectory = process.argv[2]
if (!evidenceDirectory) throw new Error('Provide an evidence directory')
await fs.mkdir(evidenceDirectory, { recursive: true })

const browser = await chromium.launch({ headless: true })
const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } })
const pageErrors = []
page.on('pageerror', error => pageErrors.push(error.message))
try {
  const response = await page.goto(base, { waitUntil: 'domcontentloaded' })
  assert.equal(response?.status(), 200)
  await page.getByText('Token claims are paused while the governed allocation is being finalized.').waitFor()
  assert.equal(await page.locator('#submitBtn').isDisabled(), true)
  assert.equal(await page.locator('#chainId').innerText(), '11155111')
  assert.equal(await page.locator('#nativeDripAmount').innerText(), 'Disabled')
  assert.equal((await page.locator('#tokenContract').innerText()).toLowerCase(), '0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6')
  assert.deepEqual(pageErrors, [])

  const [health, stats] = await Promise.all([
    page.request.get(base + '/health').then(response => response.json()),
    page.request.get(base + '/api/stats').then(response => response.json()),
  ])
  assert.equal(health.ledger_reconciled, true)
  assert.equal(health.token_claims_enabled, false)
  assert.equal(stats.token_claims_enabled, false)
  assert.equal(stats.native_drip_amount, '0')
  await page.screenshot({ path: evidenceDirectory + '/public-faucet-r26.png', fullPage: true })
  const report = {
    verified_at_utc: new Date().toISOString(),
    public_url: base,
    http_status: 200,
    chain_id: 11155111,
    ledger_reconciled: true,
    token_claims_enabled: false,
    native_drips_enabled: false,
    claim_button_disabled: true,
    observed_confirmed_drips: stats.stats?.total_drips,
    page_errors: pageErrors,
    requests_mocked: false,
    on_chain_transaction_sent: false,
  }
  await fs.writeFile(evidenceDirectory + '/public-faucet-r26-browser.json', JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify(report))
} finally {
  await browser.close()
}
