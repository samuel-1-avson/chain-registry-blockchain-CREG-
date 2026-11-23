// explorer/rpc-allowlist.js — ADR-019 Increment 1 (ECA-38).
//
// njs handler for `location = /rpc`. The route used to be an open proxy to
// the operator's Sepolia RPC (any method, any size, no rate limit). It now
// accepts only POSTed JSON-RPC 2.0 calls (single or batch of at most
// MAX_BATCH) whose every method is on ALLOWED_METHODS, then forwards the
// unchanged body to the internal `/_rpc_upstream` location.
//
// The list covers the explorer's wallet reads and the transaction lifecycle
// used by `creg testnet stake-*` (`cast send`): nonce, fee and gas estimation,
// signed raw transaction relay and receipt polling. It deliberately excludes
// expensive or state-enumerating methods such as eth_getLogs, debug_*, trace_*,
// txpool_*, admin_* and personal_*.
//
// Plain ES module so it also runs under `node --test` (rpc-allowlist.test.js).

const ALLOWED_METHODS = [
  'eth_blockNumber',
  'eth_call',
  'eth_chainId',
  'eth_estimateGas',
  'eth_feeHistory',
  'eth_gasPrice',
  'eth_getBalance',
  'eth_getBlockByHash',
  'eth_getBlockByNumber',
  'eth_getCode',
  'eth_getTransactionByHash',
  'eth_getTransactionCount',
  'eth_getTransactionReceipt',
  'eth_maxPriorityFeePerGas',
  'eth_sendRawTransaction',
  'net_version',
]

const MAX_BATCH = 20
const MAX_METHOD_CHARS = 64

function isAllowed(method) {
  return ALLOWED_METHODS.indexOf(method) !== -1
}

/**
 * Classify a raw request body.
 * Returns { ok: true } or { ok: false, status, message }.
 */
function check(body) {
  let parsed
  try {
    parsed = JSON.parse(body)
  } catch (e) {
    return { ok: false, status: 400, message: 'invalid JSON-RPC request' }
  }
  const calls = Array.isArray(parsed) ? parsed : [parsed]
  if (calls.length === 0 || calls.length > MAX_BATCH) {
    return { ok: false, status: 400, message: 'batch must contain 1..' + MAX_BATCH + ' calls' }
  }
  for (let i = 0; i < calls.length; i++) {
    const call = calls[i]
    if (call === null || typeof call !== 'object' || Array.isArray(call)) {
      return { ok: false, status: 400, message: 'invalid JSON-RPC request' }
    }
    const method = call.method
    if (typeof method !== 'string' || method.length === 0 || method.length > MAX_METHOD_CHARS) {
      return { ok: false, status: 400, message: 'invalid JSON-RPC method' }
    }
    if (!isAllowed(method)) {
      // Echo only an allowlisted character set back to the client.
      const shown = method.replace(/[^A-Za-z0-9_]/g, '?')
      return { ok: false, status: 403, message: 'method not allowed: ' + shown }
    }
  }
  return { ok: true }
}

function reply(r, status, message) {
  r.headersOut['Content-Type'] = 'application/json'
  r.return(
    status,
    JSON.stringify({ jsonrpc: '2.0', id: null, error: { code: -32601, message: message } }),
  )
}

async function proxy(r) {
  if (r.method !== 'POST') {
    reply(r, 405, 'POST only')
    return
  }
  // Bodies larger than client_body_buffer_size spill to disk and are not
  // exposed to njs; nginx caps them at client_max_body_size first anyway.
  const body = r.requestText
  if (typeof body !== 'string' || body.length === 0) {
    reply(r, 400, 'empty or oversized JSON-RPC request')
    return
  }
  const verdict = check(body)
  if (!verdict.ok) {
    reply(r, verdict.status, verdict.message)
    return
  }
  const upstream = await r.subrequest('/_rpc_upstream', { method: 'POST', body: body })
  r.headersOut['Content-Type'] = 'application/json'
  r.return(upstream.status, upstream.responseText)
}

export default { proxy, check, ALLOWED_METHODS, MAX_BATCH }
