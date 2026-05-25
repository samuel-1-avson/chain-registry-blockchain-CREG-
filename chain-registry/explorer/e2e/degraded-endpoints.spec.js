import { test, expect } from '@playwright/test'

async function mockNodeApi(page, overrides = {}) {
  await page.route('**/v1/**', async (route) => {
    const url = new URL(route.request().url())
    const override = overrides[url.pathname]

    if (override) {
      return fulfill(route, override)
    }

    switch (url.pathname) {
      case '/v1/public/chain/stats':
      case '/v1/chain/stats':
        return fulfillJson(route, {
          current_height: 42,
          validator_count: 4,
          active_validators: 4,
          package_count: 3,
          pending_tx_count: 0,
          publisher_count: 2,
          finalization_lag: 0,
        })
      case '/v1/public/bridge/status':
      case '/v1/bridge/status':
        return fulfillJson(route, {
          healthy: true,
          bridge_sync_status: 'synced',
          last_anchor_block: 128,
          last_anchor_root: `0x${'1'.repeat(64)}`,
          bridge_contract: `0x${'2'.repeat(40)}`,
          signer_address: `0x${'3'.repeat(40)}`,
          l1_chain_id: 11155111,
          commit_interval: '5m',
        })
      case '/v1/public/governance/proposals':
      case '/v1/governance/proposals':
        return fulfillJson(route, { proposals: [] })
      case '/v1/operator/pending':
      case '/v1/pending':
        return fulfillJson(route, { count: 0, packages: [] })
      case '/v1/search':
      case '/v1/public/search':
        return fulfillJson(route, { matches: [] })
      default:
        return route.fulfill({
          status: 404,
          contentType: 'text/plain',
          body: 'not found',
        })
    }
  })
}

function fulfillJson(route, body, status = 200) {
  return route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(body),
  })
}

function fulfill(route, response) {
  const body = typeof response.body === 'string'
    ? response.body
    : JSON.stringify(response.body ?? {})

  return route.fulfill({
    status: response.status ?? 200,
    contentType: response.contentType || 'application/json',
    body,
  })
}

test.describe('Degraded endpoint states', () => {
  test('bridge page distinguishes unavailable anchor history from empty data', async ({ page }) => {
    await mockNodeApi(page, {
      '/v1/public/bridge/anchors': { status: 404, contentType: 'text/plain', body: 'not found' },
    })

    await page.goto('/bridge')

    await expect(page.getByRole('heading', { name: 'Anchor history unavailable' })).toBeVisible()
    await expect(page.getByText(/historical bridge anchor data/i)).toBeVisible()
    await expect(page.getByText(/bridge configuration/i)).toBeVisible()
  })

  test('governance page distinguishes unavailable proposals endpoint from no activity', async ({ page }) => {
    await mockNodeApi(page, {
      '/v1/public/governance/proposals': { status: 404, contentType: 'text/plain', body: 'not found' },
      '/v1/governance/proposals': { status: 404, contentType: 'text/plain', body: 'not found' },
    })

    await page.goto('/governance')

    await expect(page.getByRole('heading', { name: 'Governance proposals unavailable' })).toBeVisible()
    await expect(page.getByText(/page stays available, but proposals cannot be listed/i)).toBeVisible()
  })

  test('search page surfaces unavailable search index while keeping direct lookup fallback', async ({ page }) => {
    const address = '0x0000000000000000000000000000000000000000'

    await mockNodeApi(page, {
      '/v1/public/search': { status: 404, contentType: 'text/plain', body: 'not found' },
      '/v1/search': { status: 404, contentType: 'text/plain', body: 'not found' },
    })

    await page.goto(`/search?q=${address}`)

    await expect(page.getByText(/search index unavailable/i)).toBeVisible()
    await expect(page.getByRole('link', { name: address })).toBeVisible()
  })

  test('pending page accepts grouped operator payloads with canonical-only package entries', async ({ page }) => {
    await mockNodeApi(page, {
      '/v1/operator/pending': {
        body: { count: 1, packages: ['npm/left-pad@1.0.0'] },
      },
    })

    await page.goto('/pending')

    await expect(page.getByText('npm/left-pad@1.0.0')).toBeVisible()
  })

  test('proof page reads grouped LightClientResponse fields', async ({ page }) => {
    const canonical = 'npm/express@4.18.0'
    const encodedCanonical = encodeURIComponent(canonical)

    await mockNodeApi(page, {
      [`/v1/public/packages/${encodedCanonical}/proof`]: {
        body: {
          status: 'verified',
          block_hash: `0x${'a'.repeat(64)}`,
          block_header: {
            height: 42,
            prev_hash: `0x${'b'.repeat(64)}`,
            merkle_root: 'c'.repeat(64),
            proposer_id: 'validator-1',
            timestamp: '2026-05-18T00:00:00Z',
            validator_set_hash: 'd'.repeat(64),
            vrf_output: null,
            vrf_proof: null,
          },
          proof: {
            tx_hash: 'e'.repeat(64),
            expected_root: 'c'.repeat(64),
            path: [
              { sibling_hash: 'f'.repeat(64), is_right: true },
            ],
          },
          header_chain: [],
        },
      },
    })

    await page.goto('/proof')
    await page.getByPlaceholder(/Package canonical/i).fill(canonical)
    await page.getByRole('button', { name: /Fetch proof/i }).click()

    await expect(page.getByText('Proof data')).toBeVisible()
    await expect(page.getByRole('link', { name: '#42', exact: true })).toBeVisible()
    await expect(page.getByText(/Proof path \(1 nodes\)/i)).toBeVisible()
  })

  test('block detail treats bare 64-character hashes as block hashes', async ({ page }) => {
    const hash = '64492845d276c151781028584624d3d22de533cda467d7a4718ba868298992eb'
    let heightEndpointHit = false
    let hashEndpointHit = false

    await mockNodeApi(page, {
      [`/v1/public/blocks/${hash}`]: {
        status: 400,
        body: { error: 'height must be numeric' },
      },
      [`/v1/public/blocks/hash/${hash}`]: {
        body: {
          height: 36,
          hash,
          prev_hash: '0'.repeat(64),
          timestamp: new Date().toISOString(),
          producer: 'validator-1',
          transactions: [],
          votes: [],
          finalized: true,
        },
      },
    })

    await page.route('**/v1/public/blocks/**', async (route) => {
      const path = new URL(route.request().url()).pathname
      if (path === `/v1/public/blocks/${hash}`) heightEndpointHit = true
      if (path === `/v1/public/blocks/hash/${hash}`) hashEndpointHit = true
      return route.fallback()
    })

    await page.goto(`/block/hash/${hash}`)

    await expect(page.getByRole('heading', { name: 'Block #36' })).toBeVisible()
    expect(hashEndpointHit).toBe(true)
    expect(heightEndpointHit).toBe(false)
  })
})
