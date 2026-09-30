import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import { chromium } from '@playwright/test'

const base = process.argv[2] || 'http://127.0.0.1:3007'
const evidencePath = process.argv[3]
const canonical = process.argv[4] || 'npm:escape-html@1.0.3'
const afterHeight = Number(process.argv[5] || 4)
const operation = process.argv[6] || 'publish'
if (!['publish', 'revoke'].includes(operation)) throw new Error('Operation must be publish or revoke')
if (!evidencePath) throw new Error('Usage: node capture-real-explorer-events.mjs URL EVIDENCE_JSON [CANONICAL] [STARTING_HEIGHT]')

const eventTypes = [
  'package_submitted', 'package_verified', 'package_rejected',
  'package_revoked', 'block_produced', 'validator_voted',
]
const browser = await chromium.launch({ headless: true })
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } })
const pageErrors = []
page.on('pageerror', error => pageErrors.push(error.message))
let probeFailure = null
let partialTimer = null

try {
  await page.addInitScript((types) => {
    const NativeEventSource = window.EventSource
    window.__cregEventProbe = { connections: [], events: [] }
    window.EventSource = new Proxy(NativeEventSource, {
      construct(target, args) {
        const source = Reflect.construct(target, args, target)
        const connection = { url: new URL(args[0], location.href).href, opened: false, errors: 0 }
        window.__cregEventProbe.connections.push(connection)
        source.addEventListener('open', () => { connection.opened = true })
        source.addEventListener('error', () => { connection.errors += 1 })
        for (const type of types) {
          source.addEventListener(type, event => {
            let data = event.data
            try { data = JSON.parse(data) } catch { /* keep raw event data */ }
            window.__cregEventProbe.events.push({ type, data, receivedAt: new Date().toISOString() })
          })
        }
        return source
      },
    })
  }, eventTypes)

  await page.goto(`${base}/events`, { waitUntil: 'domcontentloaded' })
  try {
    await page.waitForFunction(() => window.__cregEventProbe?.connections.some(connection => connection.opened), null, { timeout: 45_000 })
    const connections = await page.evaluate(() => window.__cregEventProbe.connections)
    await fs.writeFile(`${evidencePath}.armed.json`, JSON.stringify({ capturedAt: new Date().toISOString(), canonical, afterHeight, connections }, null, 2))
    console.log(`Browser event stream armed for ${canonical}`)
    partialTimer = setInterval(async () => {
      try {
        const partial = await page.evaluate(() => window.__cregEventProbe)
        await fs.writeFile(`${evidencePath}.partial.json`, JSON.stringify({ capturedAt: new Date().toISOString(), ...partial }, null, 2))
      } catch { /* final report records any page or stream failure */ }
    }, 3_000)
    await page.waitForFunction(({ canonical, afterHeight, operation }) => {
    const events = window.__cregEventProbe?.events || []
    const hasPackageEvent = type => events.some(event => event.type === type &&
      (event.data?.payload?.canonical === canonical || event.data?.canonical === canonical))
    const hasBlockEvent = events.some(event => event.type === 'block_produced' &&
      Number(event.data?.payload?.height ?? event.data?.height) > afterHeight)
    return (operation === 'revoke' ? hasPackageEvent('package_revoked') :
      hasPackageEvent('package_submitted') && hasPackageEvent('package_verified')) && hasBlockEvent
    }, { canonical, afterHeight, operation }, { timeout: 900_000 })
  } catch (error) { probeFailure = error }

  await page.waitForTimeout(500)
  const browserEvidence = await page.evaluate(() => ({
    connections: window.__cregEventProbe.connections,
    events: window.__cregEventProbe.events,
    renderedText: document.querySelector('main')?.innerText || '',
  }))
  const submitted = browserEvidence.events.some(event => event.type === 'package_submitted' &&
    (event.data?.payload?.canonical === canonical || event.data?.canonical === canonical))
  const verified = browserEvidence.events.some(event => event.type === 'package_verified' &&
    (event.data?.payload?.canonical === canonical || event.data?.canonical === canonical))
  const revoked = browserEvidence.events.some(event => event.type === 'package_revoked' &&
    (event.data?.payload?.canonical === canonical || event.data?.canonical === canonical))
  const block = browserEvidence.events.find(event => event.type === 'block_produced' &&
    Number(event.data?.payload?.height ?? event.data?.height) > afterHeight)
  const rendered = browserEvidence.renderedText.includes(canonical) &&
    (operation === 'revoke' ? browserEvidence.renderedText.includes('package_revoked') :
      browserEvidence.renderedText.includes('package_submitted') && browserEvidence.renderedText.includes('package_verified'))

  const report = {
    capturedAt: new Date().toISOString(),
    dataSource: 'live Phase C candidate node through the real Vite proxy; no fixtures or intercepted responses',
    explorerUrl: base,
    candidateNodeUrl: process.env.CREG_NODE_URL || 'http://127.0.0.1:18080',
    operation,
    canonical,
    startingHeight: afterHeight,
    probeError: probeFailure?.message || null,
    checks: { stream_opened: browserEvidence.connections.some(connection => connection.opened),
      ...(operation === 'revoke' ? { revoked } : { submitted, verified }),
      block_produced: Boolean(block), event_rendered_in_web_explorer: rendered, no_page_errors: pageErrors.length === 0 },
    connections: browserEvidence.connections,
    events: browserEvidence.events,
    renderedText: browserEvidence.renderedText,
    pageErrors,
  }
  await fs.writeFile(evidencePath, `${JSON.stringify(report, null, 2)}\n`)
  if (probeFailure) throw probeFailure
  assert.ok(report.checks.stream_opened, 'browser SSE did not open')
  if (operation === 'revoke') assert.ok(revoked, 'package_revoked was not delivered')
  else {
    assert.ok(submitted, 'package_submitted was not delivered')
    assert.ok(verified, 'package_verified was not delivered')
  }
  assert.ok(block, 'new block_produced event was not delivered')
  assert.ok(rendered, 'web explorer did not render the named package event')
  assert.equal(pageErrors.length, 0, JSON.stringify(pageErrors))
  console.log(JSON.stringify({ verified: true, checks: report.checks, block_height: block.data?.payload?.height ?? block.data?.height }))
} finally {
  clearInterval(partialTimer)
  await browser.close()
}
