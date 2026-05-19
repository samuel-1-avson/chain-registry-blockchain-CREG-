import { test, expect } from '@playwright/test'

const EXPECT_PWA_ASSETS = process.env.E2E_EXPECT_PWA_ASSETS === '1'

// Routes defined by explorer/src/App.jsx. Each entry is walked headlessly; we
// assert the shell renders, the route title/nav element shows, and no console
// error fires. Detail routes use a sentinel id so the page exercises its
// not-found / empty-state branch rather than requiring a seeded fixture.
const ROUTES = [
  { path: '/',             sel: 'text=Chain Registry' },
  { path: '/blocks',       sel: 'text=/Blocks?/i' },
  { path: '/block/0',      sel: 'text=/Block|height|genesis/i' },
  { path: '/tx/nonexistent-tx-hash-0000', sel: 'text=/transaction|not found|Tx/i' },
  { path: '/address/0x0000000000000000000000000000000000000000', sel: 'text=/Address|Balance|Not found/i' },
  { path: '/validator/0x0000000000000000000000000000000000000000', sel: 'text=/Validator|uptime|Not found/i' },
  { path: '/validators',   sel: 'text=/Validators?/i' },
  { path: '/packages',     sel: 'text=/Packages?/i' },
  { path: '/package/nonexistent-pkg', sel: 'text=/Package|Not found|canonical/i' },
  { path: '/publisher/00', sel: 'text=/Publisher|Address|Not found/i' },
  { path: '/pending',      sel: 'text=/Pending|Mempool/i' },
  { path: '/consensus',    sel: 'text=/Consensus|PBFT|Round/i' },
  { path: '/events',       sel: 'text=/Events?/i' },
  { path: '/network',      sel: 'text=/Network|Peers?/i' },
  { path: '/bridge',       sel: 'text=/Bridge|L1|anchor/i' },
  { path: '/governance',   sel: 'text=/Governance|Proposal/i' },
  { path: '/metrics',      sel: 'text=/Metrics|TPS|chart/i' },
  { path: '/proof',        sel: 'text=/Proof|verify/i' },
  { path: '/richlist',     sel: 'text=/Rich|Top|balance/i' },
  { path: '/reorgs',       sel: 'text=/Reorg|fork/i' },
  { path: '/search?q=test', sel: 'text=/Search|result/i' },
  { path: '/about',        sel: 'text=/About|genesis|contracts/i' },
]

test.describe('Explorer headless walk', () => {
  for (const r of ROUTES) {
    test(`renders ${r.path}`, async ({ page }) => {
      const consoleErrors = []
      page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`))
      page.on('console', (msg) => {
        if (msg.type() === 'error') consoleErrors.push(msg.text())
      })

      const resp = await page.goto(r.path, { waitUntil: 'domcontentloaded' })
      expect(resp, `no response for ${r.path}`).toBeTruthy()
      expect(resp.status(), `HTTP status for ${r.path}`).toBeLessThan(400)

      // App shell loaded — look for the global header brand text
      await expect(page.locator('text=Chain Registry').first()).toBeVisible({ timeout: 10_000 })

      // Page-specific signal (soft): at least one of the expected tokens present
      await expect(page.locator(r.sel).first()).toBeVisible({ timeout: 10_000 })

      // Filter out network-noise errors (SSE retry, fetch aborted on nav, etc.)
      // Ignore network-noise + expected 404s from sentinel detail-route fixtures.
      const realErrors = consoleErrors.filter((m) =>
        !/Failed to fetch|NetworkError|ResizeObserver|WebSocket|EventSource|aborted|status of 404|status of 5\d\d/i.test(m)
      )
      expect(realErrors, `console errors on ${r.path}:\n${realErrors.join('\n')}`).toEqual([])
    })
  }

  test('NotFound route returns the 404 view', async ({ page }) => {
    await page.goto('/this-route-does-not-exist')
    await expect(page.locator('text=/404|not found/i').first()).toBeVisible()
  })

  test('SearchBar smart-classify navigates from header', async ({ page }) => {
    await page.goto('/')
    const search = page.locator('input[type="search"], input[placeholder*="search" i]').first()
    await expect(search).toBeVisible()
    await search.fill('42')
    await search.press('Enter')
    await expect(page).toHaveURL(/\/(block\/42|search)/)
  })

  test('Wallet nav link is present and routes to /wallet', async ({ page }) => {
    await page.goto('/')
    const walletLink = page.locator('a:has-text("Wallet")').first()
    await expect(walletLink).toBeVisible()
    await walletLink.click()
    await expect(page).toHaveURL(/\/wallet/)
  })

  test('PWA manifest + service worker are served', async ({ request }) => {
    test.skip(!EXPECT_PWA_ASSETS, 'PWA asset assertions require preview/build output rather than the Vite dev server.')

    const manifest = await request.get('/manifest.webmanifest')
    expect(manifest.status()).toBe(200)
    const json = await manifest.json()
    expect(json.name).toMatch(/Chain Registry/)

    const sw = await request.get('/sw.js')
    expect(sw.status()).toBe(200)
    expect((await sw.text()).length).toBeGreaterThan(100)
  })
})
