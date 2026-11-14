import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import path from 'node:path'

const require = createRequire(path.join(process.cwd(), 'package.json'))
const { chromium } = require('playwright')
const evidenceDir = path.resolve(process.cwd(), '../../docs/testnet-heal-evidence/2026-09-27-phase-c-metrics-history-r23')
const screenshotPath = path.join(evidenceDir, 'metrics-page-r23.png')
const reportPath = path.join(evidenceDir, 'metrics-page-r23.json')
const pageErrors = []

const browser = await chromium.launch({ headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1600, height: 1200 }, deviceScaleFactor: 1 })
  page.on('pageerror', error => pageErrors.push(error.message))
  await page.goto('http://127.0.0.1:3007/metrics', { waitUntil: 'domcontentloaded' })
  await page.getByRole('heading', { name: 'Chain height' }).waitFor({ state: 'visible', timeout: 45_000 })
  await page.waitForFunction(() => document.querySelectorAll('.recharts-surface').length >= 3, { timeout: 45_000 })

  const history = await page.evaluate(async () => {
    const response = await fetch('/v1/public/metrics/history?range=30m', { cache: 'no-store' })
    return { http: response.status, body: await response.json() }
  })
  assert.equal(history.http, 200)
  assert.equal(history.body.status, 'ready')
  assert.equal(history.body.collector, 'prometheus')
  assert.equal(history.body.chain_id, 'creg-phase-c-testnet-2026-09-26-r7')
  assert.ok(history.body.sample_count >= 3)
  assert.equal(history.body.queried_series, 7)
  assert.ok(history.body.samples.some(sample => Number(sample.height) === 14))

  const headings = await page.locator('h3').allTextContents()
  const chartCount = await page.locator('.recharts-surface').count()
  assert.ok(headings.includes('Chain height'))
  assert.ok(chartCount >= 3)
  assert.deepEqual(pageErrors, [])
  await page.screenshot({ path: screenshotPath, fullPage: true })

  const report = {
    captured_at: new Date().toISOString(),
    url: 'http://127.0.0.1:3007/metrics',
    real_node_api: 'R7 O1 through the temporary SSH/IAP tunnel',
    api_http: history.http,
    collector: history.body.collector,
    history_status: history.body.status,
    chain_id: history.body.chain_id,
    network_genesis_hash: history.body.network_genesis_hash,
    sample_count: history.body.sample_count,
    queried_series: history.body.queried_series,
    observed_fields: [...new Set(history.body.samples.flatMap(sample => Object.keys(sample).filter(key => key !== 'timestamp_ms')))].sort(),
    latest_height: history.body.samples.filter(sample => sample.height != null).at(-1)?.height,
    chart_headings: headings,
    rendered_chart_count: chartCount,
    page_errors: pageErrors,
    screenshot: screenshotPath,
  }
  await import('node:fs/promises').then(fs => fs.writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`))
  console.log(JSON.stringify(report))
} finally {
  await browser.close()
}