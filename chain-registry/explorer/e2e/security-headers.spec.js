// ADR-019 Increment 1 exit gate (ECA-35, ECA-38): edge headers and /rpc policy.
//
// These assertions describe the nginx image, not the Vite dev server, so they
// run only when E2E_EXPECT_EDGE_HEADERS=1, e.g.:
//   docker build -t creg-explorer-test chain-registry/explorer
//   docker run --rm -p 3007:80 creg-explorer-test
//   E2E_BASE_URL=http://localhost:3007 E2E_EXPECT_EDGE_HEADERS=1 npx playwright test security-headers
import { test, expect } from '@playwright/test'

const EDGE = process.env.E2E_EXPECT_EDGE_HEADERS === '1'

const REQUIRED = {
  'content-security-policy': /script-src 'self'(;|$)/,
  'strict-transport-security': /max-age=\d+/,
  'x-content-type-options': /^nosniff$/,
  'x-frame-options': /^DENY$/,
  'referrer-policy': /^strict-origin-when-cross-origin$/,
  'permissions-policy': /camera=\(\)/,
}

function expectSecurityHeaders(response, label) {
  const headers = response.headers()
  for (const [name, pattern] of Object.entries(REQUIRED)) {
    expect(headers[name], `${label}: ${name}`).toMatch(pattern)
  }
  const csp = headers['content-security-policy']
  for (const directive of ["object-src 'none'", "base-uri 'none'", "frame-ancestors 'none'"]) {
    expect(csp, `${label}: ${directive}`).toContain(directive)
  }
  expect(csp, `${label}: no unsafe script sources`).not.toMatch(/script-src[^;]*'unsafe-(inline|eval)'/)
}

test.describe('edge security headers', () => {
  test.skip(!EDGE, 'set E2E_EXPECT_EDGE_HEADERS=1 against the nginx image')

  test('document, SPA deep link and asset all carry the headers', async ({ request }) => {
    const root = await request.get('/')
    expect(root.status()).toBe(200)
    expectSecurityHeaders(root, '/')

    // SPA fallback goes through `location = /index.html`, which sets its own
    // Cache-Control; that is exactly where nginx used to drop the headers.
    const deepLink = await request.get('/block/1')
    expect(deepLink.status()).toBe(200)
    expectSecurityHeaders(deepLink, '/block/1')
    expect(deepLink.headers()['cache-control']).toContain('no-store')

    const html = await root.text()
    const asset = /<script[^>]+src="([^"]+\.js)"/.exec(html)?.[1]
    expect(asset, 'built document references a script asset').toBeTruthy()
    const assetResponse = await request.get(asset)
    expect(assetResponse.status()).toBe(200)
    expectSecurityHeaders(assetResponse, asset)

    const health = await request.get('/health')
    expectSecurityHeaders(health, '/health')
  })

  test('/rpc is POST-only and method-allowlisted', async ({ request }) => {
    const get = await request.get('/rpc')
    expect(get.status()).toBe(405)

    const denied = await request.post('/rpc', {
      data: { jsonrpc: '2.0', id: 1, method: 'eth_getLogs', params: [{}] },
    })
    expect(denied.status()).toBe(403)
    expectSecurityHeaders(denied, '/rpc (denied)')

    const malformed = await request.post('/rpc', {
      headers: { 'content-type': 'application/json' },
      data: 'not json',
    })
    expect(malformed.status()).toBe(400)

    const oversized = await request.post('/rpc', {
      headers: { 'content-type': 'application/json' },
      data: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_call', params: ['x'.repeat(70 * 1024)] }),
    })
    expect(oversized.status()).toBe(413)
  })
})
