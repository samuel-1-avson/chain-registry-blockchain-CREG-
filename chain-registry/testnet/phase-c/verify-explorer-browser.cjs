// Actual browser against an actual deployed node. No route interception,
// fabricated responses, provider mocks, or package/contract writes.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { chromium } = require(process.env.CREG_BROWSER_PACKAGES || 'C:/Users/samue/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright');

async function main() {
  const [base, directory, chain, height] = process.argv.slice(2);
  if (!base || !directory || !chain || height === undefined) throw new Error('Provide base URL, evidence directory, expected chain, and height');
  fs.mkdirSync(directory, { recursive: true });
  const executablePath = process.env.CREG_CHROMIUM_EXECUTABLE || [chromium.executablePath(),
    'C:/Program Files/Google/Chrome/Application/chrome.exe',
    'C:/Program Files/BraveSoftware/Brave-Browser/Application/brave.exe',
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe'].find(file => fs.existsSync(file));
  if (!executablePath) throw new Error('No installed Chromium browser is available');
  const browser = await chromium.launch({ headless: true, executablePath });
  const context = await browser.newContext({ viewport: { width: 1440, height: 960 } });
  const report = { captured_at_utc: new Date().toISOString(), base_url: base, expected_chain: chain,
    expected_height: Number(height), javascript_errors: [], http_5xx: [], routes: [],
    writes_performed: false, response_interception_used: false };
  try {
    const page = await context.newPage();
    page.on('pageerror', error => report.javascript_errors.push(String(error)));
    page.on('response', response => {
      if (response.status() >= 500) report.http_5xx.push({ url: response.url(), status: response.status() });
    });
    const statsResponse = await context.request.get(base + '/v1/chain/stats');
    assert.equal(statsResponse.status(), 200);
    const stats = await statsResponse.json();
    assert.equal(stats.chain_id, chain); assert.equal(stats.tip_height, Number(height));
    report.node_stats = stats;
    for (const route of ['/', '/blocks', '/pending', '/validators', '/packages', '/consensus', '/bridge',
      '/events', '/network', '/metrics', '/richlist', '/reorgs', '/proof', '/wallet', '/publish']) {
      const response = await page.goto(base + route, { waitUntil: 'domcontentloaded', timeout: 30000 });
      assert.equal(response.status(), 200, route);
      await page.locator('main').waitFor({ state: 'visible', timeout: 20000 });
      await page.waitForTimeout(1200);
      const body = await page.locator('body').innerText();
      assert(body.length > 100, 'Empty application: ' + route);
      if (route === '/') {
        assert(body.includes(chain), 'Dashboard missing actual chain identity');
        assert(body.includes(String(stats.total_stake)), 'Dashboard missing real aggregate stake');
      }
      if (route === '/pending' && stats.pending_tx_count === 0) {
        assert(body.includes('Pending pool is empty'), 'Pending page did not render the public empty state');
        assert(!body.includes('Pending pool unavailable'), 'Pending page showed an operator-only endpoint error');
      }
      if (route === '/network') {
        assert(body.includes('4 / 4'), 'Network page did not render all four public validators');
        assert(!body.includes('Node list unavailable'), 'Network page showed an operator-only endpoint error');
      }
      if (route === '/metrics') {
        assert(body.includes('stored samples'), 'Metrics page did not render durable history');
        assert(!body.includes('Historical metrics unavailable'), 'Metrics page reported history unavailable');
      }
      const filename = route === '/' ? 'dashboard.png' : route.slice(1) + '.png';
      await page.screenshot({ path: path.resolve(directory, filename), fullPage: true });
      report.routes.push({ path: route, http_status: response.status(), rendered_text: body });
    }
    const rpc = await context.request.post(base + '/rpc', { data: {jsonrpc:'2.0', id:1, method:'eth_chainId', params:[]} });
    assert.equal(rpc.status(), 200); assert.equal((await rpc.json()).result, '0xaa36a7');
    report.sepolia_rpc_chain_id = 11155111;
    assert.equal(report.javascript_errors.length, 0);
    assert.equal(report.http_5xx.length, 0);
    report.status = 'passed';
  } catch (error) { report.status = 'failed'; report.error = String(error); throw error; }
  finally {
    fs.writeFileSync(path.join(directory, 'browser-verification.json'), JSON.stringify(report, null, 2) + '\n');
    await browser.close();
    console.log(JSON.stringify({ status: report.status, routes: report.routes.length,
      javascript_errors: report.javascript_errors.length, http_5xx: report.http_5xx.length, error: report.error }));
  }
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
