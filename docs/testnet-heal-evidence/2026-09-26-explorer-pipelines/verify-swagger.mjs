import { chromium } from '../../../chain-registry/explorer/node_modules/@playwright/test/index.mjs'
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
const browser = await chromium.launch({ headless: true })
const page = await browser.newPage()
const errors = []
page.on('pageerror', error => errors.push(error.message))
try {
  await page.goto('https://explorer.testnet.cregnet.dev/api-docs/', { waitUntil: 'domcontentloaded' })
  await page.waitForSelector('.swagger-ui .opblock', { timeout: 25000 })
  const operations = await page.locator('.swagger-ui .opblock').count()
  assert.ok(operations > 0)
  assert.equal(errors.length, 0, JSON.stringify(errors))
  const report = { checkedAt: new Date().toISOString(), url: page.url(), title: await page.title(),
    realOperationsRendered: operations, pageErrors: errors, verified: true }
  await fs.writeFile(process.argv[2], JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify(report))
} finally { await browser.close() }
