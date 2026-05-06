import { test, expect } from '@playwright/test'

async function mockNodeApi(page, overrides = {}) {
  await page.route('**/v1/**', async (route) => {
    const url = new URL(route.request().url())
    const override = overrides[url.pathname]

    if (override) {
      return fulfill(route, override)
    }

    switch (url.pathname) {
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
      case '/v1/governance/proposals':
        return fulfillJson(route, { proposals: [] })
      case '/v1/search':
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
      '/v1/bridge/anchors': { status: 404, contentType: 'text/plain', body: 'not found' },
    })

    await page.goto('/bridge')

    await expect(page.getByRole('heading', { name: 'Anchor history unavailable' })).toBeVisible()
    await expect(page.getByText(/historical bridge anchor data/i)).toBeVisible()
    await expect(page.getByText(/bridge configuration/i)).toBeVisible()
  })

  test('governance page distinguishes unavailable proposals endpoint from no activity', async ({ page }) => {
    await mockNodeApi(page, {
      '/v1/governance/proposals': { status: 404, contentType: 'text/plain', body: 'not found' },
    })

    await page.goto('/governance')

    await expect(page.getByRole('heading', { name: 'Governance proposals unavailable' })).toBeVisible()
    await expect(page.getByText(/page stays available, but proposals cannot be listed/i)).toBeVisible()
  })

  test('search page surfaces unavailable search index while keeping direct lookup fallback', async ({ page }) => {
    const address = '0x0000000000000000000000000000000000000000'

    await mockNodeApi(page, {
      '/v1/search': { status: 404, contentType: 'text/plain', body: 'not found' },
    })

    await page.goto(`/search?q=${address}`)

    await expect(page.getByText(/search index unavailable/i)).toBeVisible()
    await expect(page.getByRole('link', { name: address })).toBeVisible()
  })
})