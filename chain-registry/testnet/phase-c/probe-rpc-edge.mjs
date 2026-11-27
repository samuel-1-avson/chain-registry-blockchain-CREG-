#!/usr/bin/env node
// Read-only probe of CREG's public RPC surfaces (docs/rpc-remediation-plan.md, Phase 0).
//
// Checks the native CREG JSON-RPC (/rpc, /jsonrpc) and the Sepolia eth_* gateways
// (/sepolia, explorer /rpc, faucet /rpc) against the target behaviour in the plan, and
// prints PASS / FAIL / WARN / INFO for each check. FAIL means "not at target yet": until
// Phases 1 and 2 land, R-01, R-06 and R-08 are expected to fail.
//
// Every request is a read. Nothing is signed or submitted.
// Request budget for one run with default options:
//   - native node /rpc: 11 POSTs. Nodes before Phase 2 count them in the 30/min
//     "mutating" bucket (600/min JSON-RPC bucket after), and a node that trusts no
//     proxy (R-04) shares that bucket among every client.
//   - each Sepolia gateway: 10 requests, one at a time.
// --burst adds 31 POSTs to native /rpc to test R-03. While the bucket may be shared it
// can throttle native /rpc for everyone for up to 60 s, so it is off by default.
//
// Usage:
//   node probe-rpc-edge.mjs [--out FILE.json] [--burst] [--max-tip-age-min N]
//                           [--api URL|none] [--rpc URL|none] [--explorer URL|none] [--faucet URL|none]
// Exit code: 0 when nothing FAILs, 1 when something does, 2 on bad arguments.

import fs from 'node:fs/promises'
import path from 'node:path'

const USAGE =
  'Usage: node probe-rpc-edge.mjs [--out FILE.json] [--burst] [--max-tip-age-min N]\n' +
  '         [--api URL|none] [--rpc URL|none] [--explorer URL|none] [--faucet URL|none]'
const SEPOLIA_CHAIN_ID = '0xaa36a7'
const TIMEOUT_MS = 20_000
const PAUSE_MS = 150 // stays under the explorer gateway's 10 r/s limit
const USER_AGENT = 'creg-rpc-edge-probe/1'
const OVERSIZE_BYTES = 70 * 1024 // gateway cap is 64 KiB
const BURST_CALLS = 31 // one more than today's 30/min bucket
// creg_health fields that plan task 2.4 adds. Keep in step with the node when it lands.
const HEALTH_TIP_FIELDS = ['tip_timestamp', 'tip_age_secs']
const PREFLIGHT_HEADERS = {
  origin: 'https://example.org',
  'access-control-request-method': 'POST',
  'access-control-request-headers': 'content-type',
}
const DEFAULTS = {
  api: 'https://api.testnet.cregnet.dev',
  rpc: 'https://rpc.testnet.cregnet.dev',
  explorer: 'https://explorer.testnet.cregnet.dev',
  faucet: 'https://faucet.testnet.cregnet.dev',
  out: null,
  burst: false,
  maxTipAgeMin: '30',
}
const HOST_KEYS = ['api', 'rpc', 'explorer', 'faucet']

class UsageError extends Error {}

function parseArgs(argv) {
  const options = { ...DEFAULTS }
  const valued = {
    '--api': 'api',
    '--rpc': 'rpc',
    '--explorer': 'explorer',
    '--faucet': 'faucet',
    '--out': 'out',
    '--max-tip-age-min': 'maxTipAgeMin',
  }
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]
    if (arg === '--help' || arg === '-h') return { help: true }
    if (arg === '--burst') {
      options.burst = true
      continue
    }
    const key = valued[arg]
    if (!key || i + 1 >= argv.length) throw new UsageError(`unknown or incomplete argument: ${arg}`)
    options[key] = argv[++i]
  }
  options.maxTipAgeMin = Number(options.maxTipAgeMin)
  if (!Number.isFinite(options.maxTipAgeMin) || options.maxTipAgeMin <= 0) {
    throw new UsageError('--max-tip-age-min must be a positive number')
  }
  for (const key of HOST_KEYS) {
    if (options[key] === 'none') {
      options[key] = null
      continue
    }
    let url
    try {
      url = new URL(options[key])
    } catch {
      throw new UsageError(`--${key} is not a valid URL`)
    }
    if (url.protocol !== 'https:' && url.protocol !== 'http:') {
      throw new UsageError(`--${key} must be an http or https URL`)
    }
    options[key] = url.origin
  }
  return options
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const hostOf = (origin) => new URL(origin).host
const clip = (value, max) => {
  const text = String(value ?? '')
  return text.length > max ? `${text.slice(0, max)}...` : text
}
const request = (method, params, id) => ({ jsonrpc: '2.0', method, params, id })

function pickHeaders(headers) {
  const picked = {}
  for (const name of [
    'server',
    'via',
    'cf-ray',
    'content-type',
    'retry-after',
    'access-control-allow-origin',
    'access-control-allow-methods',
  ]) {
    const value = headers.get(name)
    if (value !== null) picked[name] = value
  }
  return picked
}

async function send(url, { method = 'POST', body, headers = {} } = {}, pauseMs = PAUSE_MS) {
  const started = Date.now()
  const init = {
    method,
    headers: { 'user-agent': USER_AGENT, ...headers },
    redirect: 'manual',
    signal: AbortSignal.timeout(TIMEOUT_MS),
  }
  if (body !== undefined) {
    init.body = body
    init.headers['content-type'] = 'application/json'
  }
  let outcome
  try {
    const response = await fetch(url, init)
    const text = await response.text()
    let json
    try {
      json = text ? JSON.parse(text) : undefined
    } catch {
      json = undefined
    }
    outcome = { status: response.status, ms: Date.now() - started, text, json, headers: pickHeaders(response.headers) }
  } catch (error) {
    const reason =
      error?.name === 'TimeoutError'
        ? `timeout after ${TIMEOUT_MS} ms`
        : String(error?.cause?.code ?? error?.message ?? error)
    outcome = { error: reason, ms: Date.now() - started }
  }
  if (pauseMs) await sleep(pauseMs)
  return outcome
}

const rpc = (url, method, params = [], id = 1) => send(url, { body: JSON.stringify(request(method, params, id)) })

function describe(res) {
  if (res.error) return `error: ${res.error}`
  let detail
  if (Array.isArray(res.json)) {
    const firstError = res.json.find((item) => item?.error)?.error
    detail = `array[${res.json.length}]${firstError ? `, first error ${firstError.code}` : ''}`
  } else if (res.json?.error) {
    detail = `rpc error ${res.json.error.code} ${clip(res.json.error.message, 60)}`
  } else if (res.json && typeof res.json === 'object' && 'result' in res.json) {
    detail = `result ${clip(JSON.stringify(res.json.result), 70)}`
  } else {
    detail = res.text ? `body ${clip(res.text, 70)}` : 'empty body'
  }
  return `HTTP ${res.status}, ${detail}`
}

function headerSummary(res) {
  if (!res.headers) return describe(res)
  const h = res.headers
  return [
    `server ${h.server ?? '-'}`,
    `via ${h.via ?? '-'}`,
    `cloudflare ${h['cf-ray'] ? 'yes' : 'no'}`,
    `allow-origin ${h['access-control-allow-origin'] ?? '-'}`,
  ].join(', ')
}

function rejectedOnly(json) {
  const items = Array.isArray(json) ? json : [json]
  return (
    items.length > 0 &&
    items.every((item) => item && typeof item === 'object' && item.error && !('result' in item))
  )
}

const results = []
const gatewayHeads = []
const verdictFor = (ok) => (ok ? 'PASS' : 'FAIL')
function record(finding, surface, check, verdict, observed) {
  results.push({ finding, surface, check, verdict, observed })
}

// Sepolia eth_* gateway. Target: the ADR-019 ECA-38 allowlist is enforced.
async function probeGateway(surface, url, getExpectation) {
  const get = await send(url, { method: 'GET' })
  if (getExpectation === '405') {
    record('R-01', surface, 'GET refused (405)', verdictFor(get.status === 405), describe(get))
  } else {
    const ok = get.status === 200 && /allowlisted/i.test(get.text ?? '')
    record('R-01', surface, 'GET descriptor says allowlisted', verdictFor(ok), describe(get))
  }

  const preflight = await send(url, { method: 'OPTIONS', headers: PREFLIGHT_HEADERS })
  record('R-09', surface, 'CORS preflight', 'INFO', `${describe(preflight)}; ${headerSummary(preflight)}`)

  const chainId = await rpc(url, 'eth_chainId')
  record('-', surface, 'eth_chainId', verdictFor(chainId.json?.result === SEPOLIA_CHAIN_ID), describe(chainId))
  record('R-05', surface, 'upstream response headers', 'INFO', headerSummary(chainId))

  const head = await rpc(url, 'eth_blockNumber')
  const headOk = typeof head.json?.result === 'string' && /^0x[0-9a-f]+$/i.test(head.json.result)
  record('-', surface, 'eth_blockNumber', verdictFor(headOk), describe(head))
  if (headOk) gatewayHeads.push({ surface, height: Number.parseInt(head.json.result, 16) })

  const client = await rpc(url, 'web3_clientVersion')
  record('R-01', surface, 'web3_clientVersion denied', verdictFor(client.status === 403), describe(client))

  const logs = await rpc(url, 'eth_getLogs', [{ fromBlock: 'latest', toBlock: 'latest' }])
  record('R-01', surface, 'eth_getLogs denied', verdictFor(logs.status === 403), describe(logs))

  const mixed = await send(url, {
    body: JSON.stringify([
      request('eth_chainId', [], 1),
      request('eth_getLogs', [{ fromBlock: 'latest', toBlock: 'latest' }], 2),
    ]),
  })
  record('R-01', surface, 'batch with denied method', verdictFor(mixed.status === 403), describe(mixed))

  const oversizedBatch = await send(url, {
    body: JSON.stringify(Array.from({ length: 21 }, (_, i) => request('eth_chainId', [], i + 1))),
  })
  record('R-14', surface, 'batch of 21 refused', verdictFor(oversizedBatch.status === 400), describe(oversizedBatch))

  // An upstream node also answers 400 to bad JSON, so require the gateway's own message
  // (explorer/rpc-allowlist.js) to tell the gateway's refusal from the upstream's.
  const malformed = await send(url, { body: '{"jsonrpc":"2.0",' })
  const refusedByGateway =
    malformed.status === 400 && /invalid JSON-RPC request/i.test(String(malformed.json?.error?.message ?? ''))
  record('R-01', surface, 'malformed body refused by gateway', verdictFor(refusedByGateway), describe(malformed))

  const oversize = await send(url, {
    body: JSON.stringify(request('eth_chainId', ['x'.repeat(OVERSIZE_BYTES)], 1)),
  })
  record('R-01', surface, '70 KiB body refused (413)', verdictFor(oversize.status === 413), describe(oversize))
}

// Native CREG JSON-RPC. Target: the Phase 2 behaviour in the plan.
async function probeNative(origin, options) {
  const url = `${origin}/rpc`
  const surface = `${hostOf(origin)}/rpc`

  const get = await send(url, { method: 'GET' })
  record('-', surface, 'GET descriptor', 'INFO', describe(get))

  const preflight = await send(url, { method: 'OPTIONS', headers: PREFLIGHT_HEADERS })
  record('R-09', surface, 'CORS preflight', 'INFO', `${describe(preflight)}; ${headerSummary(preflight)}`)

  const chainId = await rpc(url, 'creg_chainId')
  const chainIdOk = typeof chainId.json?.result === 'string' && chainId.json.result.length > 0
  record('-', surface, 'creg_chainId', verdictFor(chainIdOk), describe(chainId))

  const blockNumber = await rpc(url, 'creg_blockNumber')
  const blockNumberOk = /^0x[0-9a-f]+$/i.test(String(blockNumber.json?.result ?? ''))
  record('-', surface, 'creg_blockNumber', verdictFor(blockNumberOk), describe(blockNumber))

  const health = await rpc(url, 'creg_health')
  const status = health.json?.result
  // Blocks are produced only when work exists, so an old tip is "idle" unless the
  // node reports pending work (Phase 2 adds pending_pool_size to creg_health).
  const pending = typeof status?.pending_pool_size === 'number' ? status.pending_pool_size : null
  record('-', surface, 'creg_health status ok', verdictFor(status?.status === 'ok'), describe(health))
  if (status && typeof status === 'object') {
    const sync = status.validator_set_sync ?? {}
    record('R-06', surface, 'validator-set sync', 'INFO', `state ${sync.state ?? 'n/a'}, last_error ${sync.last_error ?? 'none'}`)
    const hasTipFields = HEALTH_TIP_FIELDS.some((field) => field in status)
    record('R-06', surface, 'creg_health reports tip age', verdictFor(hasTipFields), `fields: ${Object.keys(status).join(', ')}`)
  } else {
    record('R-06', surface, 'creg_health reports tip age', 'FAIL', describe(health))
  }

  const tip = await rpc(url, 'creg_getBlockByNumber', ['latest'])
  const header = tip.json?.result?.header
  const tipAgeMin = header?.timestamp ? (Date.now() - Date.parse(header.timestamp)) / 60_000 : Number.NaN
  if (Number.isFinite(tipAgeMin)) {
    const stale = tipAgeMin > options.maxTipAgeMin
    const verdict = stale && pending !== null && pending > 0 ? 'WARN' : 'INFO'
    const work = pending === null ? 'pending work not reported' : `${pending} pending`
    const reading = stale ? (pending > 0 ? 'possible stall' : 'idle or unknown') : 'recent'
    record(
      'R-06',
      surface,
      'tip block age',
      verdict,
      `height ${header.height}, ${header.timestamp}, ${tipAgeMin.toFixed(1)} min old, ${work} (${reading})`,
    )
  } else {
    record('R-06', surface, 'tip block age', 'FAIL', describe(tip))
  }

  const ethOnNative = await rpc(url, 'eth_chainId')
  record('R-16', surface, 'eth_* namespace', 'INFO', describe(ethOnNative))

  const batch = await send(url, {
    body: JSON.stringify([request('creg_chainId', [], 1), request('creg_blockNumber', [], 2)]),
  })
  const batchOk =
    Array.isArray(batch.json) &&
    batch.json.length === 2 &&
    batch.json.every((item) => item && typeof item === 'object' && 'result' in item)
  record('R-08', surface, 'batch answered', verdictFor(batchOk), describe(batch))

  const positional = await send(url, { body: '["2.0","creg_chainId"]' })
  record('R-08', surface, 'positional array rejected', verdictFor(rejectedOnly(positional.json)), describe(positional))

  const notification = await send(url, {
    body: JSON.stringify({ jsonrpc: '2.0', method: 'creg_blockNumber', params: [] }),
  })
  const silent = notification.status === 204 || (notification.status === 200 && (notification.text ?? '') === '')
  record('R-08', surface, 'notification gets no reply', verdictFor(silent), describe(notification))

  const noVersion = await send(url, { body: JSON.stringify({ method: 'creg_blockNumber', params: [], id: 1 }) })
  record('R-08', surface, 'missing jsonrpc rejected', verdictFor(noVersion.json?.error?.code === -32600), describe(noVersion))

  return chainIdOk ? chainId.json.result : null
}

async function probeNativeAlias(url, surface, expectedChainId) {
  const res = await rpc(url, 'creg_chainId')
  const ok = expectedChainId !== null && res.json?.result === expectedChainId
  record('-', surface, 'creg_chainId matches', verdictFor(ok), describe(res))
}

// R-03: read-only RPC must allow more than 30 calls a minute, and a 429 must be JSON-RPC.
async function probeBurst(origin) {
  const url = `${origin}/rpc`
  const surface = `${hostOf(origin)}/rpc`
  let ok = 0
  let first429 = null
  for (let i = 1; i <= BURST_CALLS; i++) {
    const res = await send(url, { body: JSON.stringify(request('creg_blockNumber', [], i)) }, 0)
    if (res.status === 200) ok++
    else if (res.status === 429 && !first429) first429 = { at: i, res }
  }
  const summary = `${ok}/${BURST_CALLS} returned 200${first429 ? `, first 429 at call ${first429.at}` : ''}`
  record('R-03', surface, `${BURST_CALLS} reads in a minute allowed`, verdictFor(ok === BURST_CALLS), summary)
  if (first429) {
    const json = first429.res.json
    const jsonRpc = json?.jsonrpc === '2.0' && typeof json?.error?.code === 'number'
    record('R-03', surface, '429 is a JSON-RPC error', verdictFor(jsonRpc), describe(first429.res))
  }
}

function printReport(report) {
  const width = Math.max(12, ...report.results.map((r) => r.surface.length))
  const checkWidth = Math.max(20, ...report.results.map((r) => r.check.length))
  for (const r of report.results) {
    console.log(`${r.verdict.padEnd(4)}  ${r.finding.padEnd(4)}  ${r.surface.padEnd(width)}  ${r.check.padEnd(checkWidth)}  ${r.observed}`)
  }
  const counts = report.summary.counts
  console.log('')
  console.log(`PASS ${counts.PASS ?? 0}, FAIL ${counts.FAIL ?? 0}, WARN ${counts.WARN ?? 0}, INFO ${counts.INFO ?? 0}`)
  if (report.summary.failingFindings.length) {
    console.log(`Findings not at target: ${report.summary.failingFindings.join(', ')}`)
  }
}

async function main() {
  if (typeof fetch !== 'function' || typeof AbortSignal.timeout !== 'function') {
    console.error('Node 18 or newer is required.')
    return 2
  }
  let options
  try {
    options = parseArgs(process.argv.slice(2))
  } catch (error) {
    if (!(error instanceof UsageError)) throw error
    console.error(error.message)
    console.error(USAGE)
    return 2
  }
  if (options.help) {
    console.log(USAGE)
    return 0
  }
  if (!HOST_KEYS.some((key) => options[key])) {
    console.error('Every host is set to none; nothing to probe.')
    return 2
  }

  const startedAt = new Date().toISOString()

  const nativeOrigin = options.api ?? options.rpc
  if (nativeOrigin) {
    const chainId = await probeNative(nativeOrigin, options)
    await probeNativeAlias(`${nativeOrigin}/jsonrpc`, `${hostOf(nativeOrigin)}/jsonrpc`, chainId)
    if (options.api && options.rpc) {
      await probeNativeAlias(`${options.rpc}/rpc`, `${hostOf(options.rpc)}/rpc`, chainId)
    }
  }

  const gateways = []
  if (options.api) gateways.push([`${hostOf(options.api)}/sepolia`, `${options.api}/sepolia`, 'descriptor'])
  if (options.rpc) gateways.push([`${hostOf(options.rpc)}/sepolia`, `${options.rpc}/sepolia`, 'descriptor'])
  if (options.explorer) gateways.push([`${hostOf(options.explorer)}/rpc`, `${options.explorer}/rpc`, '405'])
  if (options.faucet) gateways.push([`${hostOf(options.faucet)}/rpc`, `${options.faucet}/rpc`, 'descriptor'])
  for (const [surface, url, getExpectation] of gateways) await probeGateway(surface, url, getExpectation)

  if (gatewayHeads.length > 1) {
    const heights = gatewayHeads.map((head) => head.height)
    const spread = Math.max(...heights) - Math.min(...heights)
    const list = gatewayHeads.map((head) => `${head.surface} ${head.height}`).join(', ')
    record('R-05', 'all gateways', 'Sepolia head spread', 'INFO', `${spread} blocks (${list})`)
  }

  if (options.burst && nativeOrigin) await probeBurst(nativeOrigin)

  const counts = {}
  for (const r of results) counts[r.verdict] = (counts[r.verdict] ?? 0) + 1
  const failingFindings = [...new Set(results.filter((r) => r.verdict === 'FAIL').map((r) => r.finding))].sort()
  const report = {
    tool: USER_AGENT,
    startedAt,
    finishedAt: new Date().toISOString(),
    hosts: Object.fromEntries(HOST_KEYS.map((key) => [key, options[key]])),
    burst: options.burst,
    maxTipAgeMin: options.maxTipAgeMin,
    summary: { counts, failingFindings },
    results,
  }

  printReport(report)
  if (options.out) {
    await fs.mkdir(path.dirname(path.resolve(options.out)), { recursive: true })
    await fs.writeFile(options.out, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' })
    console.log(`Wrote ${options.out}`)
  }
  return counts.FAIL ? 1 : 0
}

process.exitCode = await main()
