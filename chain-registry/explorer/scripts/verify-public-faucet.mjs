import { chromium } from '@playwright/test'
import fs from 'node:fs/promises'
import assert from 'node:assert/strict'

const directory = process.argv[2]
const base = 'https://faucet.testnet.cregnet.dev'
if (!directory) throw new Error('Provide an evidence directory')
await fs.mkdir(directory, { recursive: true })
const endpoints = {}
for (const route of ['/health', '/api/network', '/api/stats', '/api/challenge']) {
  const response = await fetch(base + route)
  assert.equal(response.status, 200, route)
  endpoints[route] = await response.json()
}
assert.equal(endpoints['/health'].mode, 'real')
assert.equal(endpoints['/health'].status, 'healthy')
assert.equal(endpoints['/api/network'].chain_id, 11155111)
assert.equal(endpoints['/api/network'].token_contract.toLowerCase(), '0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6')
const rpc = await fetch(base + '/rpc', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_chainId', params: [] }) })
assert.equal(rpc.status, 200)
const chain = await rpc.json()
assert.equal(Number.parseInt(chain.result, 16), 11155111)
const browser = await chromium.launch({ headless: true })
const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } })
const errors = []
const serverErrors = []
page.on('pageerror', error => errors.push(error.message))
page.on('response', response => { if (response.status() >= 500) serverErrors.push({ url: response.url(), status: response.status() }) })
try {
  await page.goto(base, { waitUntil: 'networkidle' })
  const text = await page.locator('body').innerText()
  await fs.writeFile(directory + '/faucet-browser-observed.json', JSON.stringify({ text, errors, serverErrors }, null, 2))
  await page.screenshot({ path: directory + '/faucet-public.png', fullPage: true })
  assert.equal(await page.locator('#chainId').innerText(), '11155111', 'Live page chain ID differs from Sepolia')
  assert.equal((await page.locator('#tokenContract').innerText()).toLowerCase(), endpoints['/api/network'].token_contract.toLowerCase())
  assert.ok(text.includes('1,000') || text.includes('1000'), 'Drip amount is missing')
  assert.equal(errors.length, 0, JSON.stringify(errors))
  assert.equal(serverErrors.length, 0, JSON.stringify(serverErrors))
  await page.screenshot({ path: directory + '/faucet-public.png', fullPage: true })
  await fs.writeFile(directory + '/faucet-browser-verification.json', JSON.stringify({ timestamp_utc: new Date().toISOString(), base, status: 'passed', endpoints, rpc_chain_id: 11155111, errors, serverErrors, body: text, requests_mocked: false }, null, 2) + '\n')
  console.log(JSON.stringify({ status: 'passed', public_faucet: base, rpc_chain_id: 11155111, errors, serverErrors }))
} finally { await browser.close() }
