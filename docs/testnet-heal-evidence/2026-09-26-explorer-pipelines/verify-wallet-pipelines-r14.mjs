import { chromium } from '../../../chain-registry/explorer/node_modules/@playwright/test/index.mjs'
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'

const [evidencePath, base = 'http://127.0.0.1:3007', expectedChain = 'creg-phase-c-testnet-2026-09-26-r7'] = process.argv.slice(2)
if (!evidencePath) throw new Error('Usage: node verify-wallet-pipelines-r14.mjs EVIDENCE.json [BASE_URL] [CHAIN_ID]')

const browser = await chromium.launch({ headless: true })
const page = await browser.newPage()
const errors = []
const failedRequests = []
const specResponses = []
page.on('pageerror', error => errors.push(error.message))
page.on('requestfailed', request => failedRequests.push({ url: request.url(), error: request.failure()?.errorText }))
page.on('response', async response => {
  const url = new URL(response.url())
  if (url.origin !== new URL(base).origin || !/^\/(chain-spec\.json|phase-c\/.*\/chain-spec\.json)$/.test(url.pathname)) return
  try {
    specResponses.push({ path: url.pathname, status: response.status(), spec: await response.json() })
  } catch (error) {
    specResponses.push({ path: url.pathname, status: response.status(), error: error.message })
  }
})

try {
  await page.goto(`${base}/wallet`, { waitUntil: 'domcontentloaded' })
  await page.waitForTimeout(1200)
  const statsResponse = await page.evaluate(async () => {
    const response = await fetch('/v1/public/chain/stats', { cache: 'no-store' })
    return { status: response.status, body: await response.json() }
  })
  const specPath = expectedChain === 'creg-phase-c-testnet-2026-09-26-r7'
    ? '/phase-c/r7/chain-spec.json'
    : '/chain-spec.json'
  const specResponse = await page.evaluate(async path => {
    const response = await fetch(path, { cache: 'no-store' })
    return { status: response.status, body: await response.json() }
  }, specPath)
  const rpcResponse = await page.evaluate(async () => {
    const response = await fetch('/rpc', { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', method: 'eth_chainId', params: [], id: 14 }) })
    return { status: response.status, body: await response.json() }
  })

  assert.equal(statsResponse.status, 200)
  assert.equal(statsResponse.body.chain_id, expectedChain)
  assert.equal(statsResponse.body.spec_signature_verified, true)
  assert.equal(specResponse.status, 200)
  assert.equal(specResponse.body.chain_id, expectedChain)
  assert.equal(specResponse.body.l1.chain_id, 11155111)
  assert.ok(/^0x[0-9a-fA-F]{40}$/.test(specResponse.body.contracts.creg_token))
  assert.ok(/^0x[0-9a-fA-F]{40}$/.test(specResponse.body.contracts.staking))
  assert.equal(rpcResponse.status, 200)
  assert.equal(rpcResponse.body.result, '0xaa36a7')
  assert.equal(errors.length, 0, JSON.stringify(errors))

  const result = {
    capturedAt: new Date().toISOString(),
    base,
    expectedChain,
    liveStats: { status: statsResponse.status, chain_id: statsResponse.body.chain_id,
      height: statsResponse.body.tip_height, genesis_hash: statsResponse.body.genesis_hash,
      spec_signature_verified: statsResponse.body.spec_signature_verified },
    spec: { path: specPath, status: specResponse.status, chain_id: specResponse.body.chain_id,
      token: specResponse.body.contracts.creg_token, staking: specResponse.body.contracts.staking,
      registry: specResponse.body.contracts.registry },
    rpc: { status: rpcResponse.status, chain_id: rpcResponse.body.result },
    pageErrors: errors,
    failedRequests,
    browserSpecResponses: specResponses.map(({ path, status, spec, error }) => ({ path, status, chain_id: spec?.chain_id, error })),
    verified: true,
    source: 'Live HTTP/API and Sepolia RPC responses, no interception or fixtures',
  }
  await fs.writeFile(evidencePath, JSON.stringify(result, null, 2) + '\n')
  console.log(JSON.stringify({ verified: true, chain: expectedChain, height: statsResponse.body.tip_height,
    spec: specPath, rpc: rpcResponse.body.result, pageErrors: errors.length, failedRequests: failedRequests.length }))
} catch (error) {
  await fs.writeFile(evidencePath.replace(/\.json$/, '.failure.json'), JSON.stringify({
    error: error.message, errors, failedRequests, specResponses,
  }, null, 2) + '\n')
  throw error
} finally {
  await browser.close()
}
