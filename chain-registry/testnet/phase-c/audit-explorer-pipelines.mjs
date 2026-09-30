// Read-only evidence from real node and explorer services. No fixture responses.
import fs from 'node:fs/promises'
import path from 'node:path'

const directory = process.argv[2]
if (!directory) throw new Error('Usage: node audit-explorer-pipelines.mjs EVIDENCE_DIRECTORY')
await fs.mkdir(directory, { recursive: true })
const bases = {
  publicExplorer: 'https://explorer.testnet.cregnet.dev',
  publicApi: 'https://api.testnet.cregnet.dev',
  candidate: 'http://127.0.0.1:18080',
}
const paths = ['/', '/v1/public/health', '/v1/public/chain/stats', '/v1/chain/stats',
  '/v1/public/blocks?limit=5', '/v1/public/blocks/0', '/v1/public/blocks/1',
  '/v1/public/blocks/4', '/v1/public/packages?limit=5',
  '/v1/public/packages/npm%3Ais-number%407.0.0',
  '/v1/public/packages/npm%3Ais-number%407.0.0/proof',
  '/v1/public/packages/npm%3Ais-number%407.0.0/intelligence',
  '/v1/public/transactions/npm%3Ais-number%407.0.0',
  '/v1/validator/registrations', '/v1/validator/consensus/state',
  '/v1/operator/runtime/config', '/v1/operator/nodes', '/v1/operator/pending',
  '/v1/operator/p2p/status', '/v1/operator/metrics/history?range=1h',
  '/v1/public/bridge/status', '/v1/public/bridge/anchors',
  '/v1/public/governance/proposals', '/v1/public/search?q=is-number',
  '/v1/public/reorgs', '/v1/public/richlist']
const results = await Promise.all(Object.entries(bases).map(async ([name, base]) => {
  const requests = await Promise.all(paths.map(async (endpoint) => {
    try {
      const response = await fetch(base + endpoint, { signal: AbortSignal.timeout(20000), cache: 'no-store' })
      const contentType = response.headers.get('content-type') || ''
      const body = await response.text()
      let payload = body.slice(0, 3000)
      if (contentType.includes('json')) payload = JSON.parse(body)
      return { path: endpoint, status: response.status, contentType, payload }
    } catch (error) { return { path: endpoint, error: error.message } }
  }))
  const result = { base, capturedAt: new Date().toISOString(), requests }
  await fs.writeFile(path.join(directory, name + '.json'), JSON.stringify(result, null, 2) + '\n', { flag: 'wx' })
  return [name, result]
}))
for (const [name, result] of results) {
  console.log(name, JSON.stringify(result.requests.map(({ path, status, error }) => ({ path, status, error }))))
  const stats = result.requests.find(r => r.path === '/v1/chain/stats')?.payload
  console.log(name, 'identity', JSON.stringify(stats))
}
