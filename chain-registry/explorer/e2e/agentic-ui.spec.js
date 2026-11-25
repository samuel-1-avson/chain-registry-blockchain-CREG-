import { test, expect } from '@playwright/test'

const BASE_URL = process.env.E2E_BASE_URL || 'http://127.0.0.1:4173'
test.use({ baseURL: BASE_URL })

const SAMPLE_TX_HASH = `0x${'ab'.repeat(32)}`
const SAMPLE_DIGEST = 'cd'.repeat(32)
const EVIL_ENV = `<img src=x onerror="(window.__xss=true)">\u202E${'A'.repeat(4096)}`

async function installCommonApiMocks(page, { eventsHandler } = {}) {
  await page.route('**/v1/**', async (route) => {
    const url = new URL(route.request().url())
    const path = url.pathname

    if (path === '/v1/public/chain/stats') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          chain_id: 'creg-test',
          tip_height: 42,
          current_height: 42,
          finalized_height: 41,
          finalization_lag: 1,
          finalization_source: 'pbft',
          active_validators: 4,
          package_count: 10,
        }),
      })
    }

    if (path === '/v1/index/quarantine-queue') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          server_time_unix: 1_700_000_000,
          truncated: false,
          entries: [
            {
              canonical: 'npm:suspicious-pkg@2.0.0',
              content_hash: 'aa'.repeat(32),
              reason_code: 'install_hook_abuse',
              opened_height: 40,
              opened_at_unix: 1_699_990_000,
              expires_at_unix: 1_700_003_661,
              committee: {
                effective_size: 4,
                approvals: 1,
                quarantine_reviews: 3,
                rejections: 0,
                abstentions: 0,
                hybrid_signers: 4,
                classical_signers: 0,
                model_families: { claude: 2, gemini: 2 },
              },
            },
          ],
        }),
      })
    }

    if (path === '/v1/index/bounties/leaderboard') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          truncated: false,
          entries: [
            {
              l1_chain_id: 11155111,
              prover_address: `0x${'12'.repeat(20)}`,
              awards: 3,
              matched_witnesses: 3,
              prover_bonus_wei: '2500000000000000000',
              committee_reserved_wei: '7500000000000000000',
              committee_distributed_wei: '0',
              latest_award: {
                l1_block_number: 700100,
                l1_block_hash: `0x${'ef'.repeat(32)}`,
                l1_tx_hash: SAMPLE_TX_HASH,
                log_index: 2,
                witness_digest: SAMPLE_DIGEST,
                package_hash: '99'.repeat(32),
                canonical: 'npm:evil@1.0.0',
                witness_height: 41,
              },
            },
          ],
        }),
      })
    }

    if (path === '/v1/index/committee-diversity') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          epoch: 0,
          validator_set_hash: '77'.repeat(32),
          observations: 12,
          approvals: 30,
          quarantine_reviews: 6,
          rejections: 2,
          abstentions: 2,
          hybrid_signers: 38,
          classical_signers: 2,
          model_families: [
            { model_family: 'claude', votes: 18 },
            { model_family: 'gemini', votes: 20 },
          ],
          model_families_truncated: false,
        }),
      })
    }

    if (path === '/v1/validator/registrations') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ registrations: [] }),
      })
    }

    if (path === '/v1/validator/consensus/state') {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ validators: [] }),
      })
    }

    if (path.startsWith('/v1/public/packages/')) {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          canonical: 'npm:evil@1.0.0',
          version: '1.0.0',
          publisher: '01'.repeat(32),
          status: 'revoked',
          revoked: true,
          revocation_reason: 'Committee QC revocation after Class B trigger',
        }),
      })
    }

    if (path.endsWith('/status') && path.startsWith('/v1/public/v4/packages/')) {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          schema: 'creg.package-view.v4',
          canonical: 'npm:evil@1.0.0',
          package: 'npm:evil',
          version: '1.0.0',
          content_hash: '44'.repeat(32),
          quorum: 'v4_quorum',
          status: {
            state: 'revoked',
            height: 41,
            authority: 'committee_quorum',
            reason: 'Trigger witness confirmed exfiltration',
            witness_digest: SAMPLE_DIGEST,
            certificate_digest: '55'.repeat(32),
          },
          committee: {
            effective_size: 4,
            approvals: 0,
            quarantine_reviews: 0,
            rejections: 4,
            abstentions: 0,
            hybrid_signers: 4,
            classical_signers: 0,
            model_families: { gemini: 2, claude: 2 },
          },
          witness_digests: [SAMPLE_DIGEST],
        }),
      })
    }

    if (path.endsWith('/witnesses') && path.startsWith('/v1/public/v4/packages/')) {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          schema: 'creg.witness-bundle.v1',
          canonical: 'npm:evil@1.0.0',
          witnesses: [
            {
              digest: SAMPLE_DIGEST,
              package_canonical: 'npm:evil@1.0.0',
              content_hash: '44'.repeat(32),
              prover_id: 'validator-1',
              prover_key_id: '66'.repeat(16),
              hybrid: true,
              env_overrides: [['MALICIOUS_ENV', EVIL_ENV]],
              clock_offset_secs: 3600,
              entry_function: 'postinstall',
              args: ['--steal-env'],
              violations: [{ code: 'KB-CANARY-EXFIL', title: 'Canary secret exfiltration', unknown: null }],
              graph_canonical_digest: '88'.repeat(32),
              verification: { kind: 'on_chain_certified', height: 41 },
            },
          ],
        }),
      })
    }

    if (path.startsWith('/v1/index/packages/') && path.endsWith('/evidence-history')) {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          canonical: 'npm:evil@1.0.0',
          next_before: null,
          entries: [
            {
              height: 41,
              tx_index: 0,
              source_tx_type: 'revoke_v2',
              status: 'revoked',
              revocation_authority: 'committee_quorum',
              reason: 'Trigger witness confirmed exfiltration',
              witness_digest: SAMPLE_DIGEST,
              certificate_digest: '55'.repeat(32),
            },
          ],
        }),
      })
    }

    if (path === '/v1/index/events' && eventsHandler) {
      return eventsHandler(route, url)
    }

    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ ok: true }),
    })
  })
}

test('eco-adv-11 & evidence modules: witness inspector, timeline, quarantine queue, bounties, telemetry, and SSE resume', async ({ page }) => {
  const pageErrors = []
  page.on('pageerror', (err) => pageErrors.push(err.message))

  const sseRequests = []
  await installCommonApiMocks(page, {
    eventsHandler: async (route, url) => {
      const after = url.searchParams.get('after')
      sseRequests.push(after)
      if (!after) {
        return route.fulfill({
          status: 200,
          contentType: 'text/event-stream',
          body: `id: 42.0\nevent: package_quarantined\ndata: ${JSON.stringify({
            id: '42.0',
            kind: 'package_quarantined',
            height: 42,
            tx_index: 0,
            canonical: 'npm:suspicious-pkg@2.0.0',
            data: { reason: 'quarantine review' },
          })}\n\n`,
        })
      }
      return route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: `id: 42.1\nevent: block_committed\ndata: ${JSON.stringify({
          id: '42.1',
          kind: 'block_committed',
          height: 43,
          tx_index: 0,
          canonical: 'npm:good-pkg@1.0.0',
          data: {},
        })}\n\n`,
      })
    },
  })

  // 1. PackageDetail: Evidence & Timeline + eco-adv-11 Trigger Witnesses
  await page.goto('/package/npm%3Aevil%401.0.0')
  await expect(page.getByRole('button', { name: 'Evidence & Timeline' })).toBeVisible()
  await page.getByRole('button', { name: 'Evidence & Timeline' }).click()
  await expect(page.getByText('creg-vote-v4 quorum')).toBeVisible()
  await expect(page.getByText('Status & Revocation Timeline')).toBeVisible()

  await page.getByRole('button', { name: 'Trigger Witnesses' }).click()
  await expect(page.getByText('OnChainCertified#41')).toBeVisible()
  await expect(page.getByText('Canary secret exfiltration')).toBeVisible()
  await expect(page.getByText(`creg evidence verify-witness ${SAMPLE_DIGEST}`)).toBeVisible()
  // eco-adv-11 assertions: payload rendered inert as text, truncated, bidi stripped, zero injected DOM imgs
  await expect(page.locator('img[src="x"]')).toHaveCount(0)
  const xssFlag = await page.evaluate(() => window.__xss)
  expect(xssFlag).toBeUndefined()
  const untrustedText = await page.locator('[data-untrusted="true"]', { hasText: '<img src=x' }).first().textContent()
  expect(untrustedText).toBeTruthy()
  expect(untrustedText).not.toContain('\u202E')
  expect(untrustedText.length).toBeLessThan(200)

  // 2. Quarantine queue (server-clock countdown & hard-block banner)
  await page.goto('/quarantine')
  await expect(page.getByText(/Fails closed to REVOKED at expiry/i)).toBeVisible()
  await expect(page.getByText('npm:suspicious-pkg@2.0.0')).toBeVisible()
  await expect(page.getByText('1h 1m')).toBeVisible()

  // 3. Bounty leaderboard (ECA-42 disclosure + hex-validated L1 link)
  await page.goto('/bounties')
  await expect(page.getByText(/reserved, not distributed/i).first()).toBeVisible()
  await expect(page.locator(`a[href="https://sepolia.etherscan.io/tx/${SAMPLE_TX_HASH}"]`)).toBeVisible()

  // 4. Committee telemetry on Validators page
  await page.goto('/validators')
  await expect(page.getByText('Committee Diversity & Telemetry')).toBeVisible()
  await expect(page.getByText('Hybrid 38 / Classical 2')).toBeVisible()

  // 5. SSE v2 reconnect with ?after=42.0
  await page.goto('/events')
  await expect(page.getByText('#42.0')).toBeVisible()
  await expect(page.getByText('#42.1')).toBeVisible({ timeout: 10_000 })
  expect(sseRequests).toContain('42.0')

  expect(pageErrors).toEqual([])
})
