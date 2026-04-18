// Chain Registry node API client.
// Single source of truth for /v1/* calls made by the explorer.
// Every helper returns parsed JSON or throws ApiError with the status code.

const API_BASE = import.meta.env.VITE_API_BASE || ''

export class ApiError extends Error {
  constructor(status, url, body) {
    super(`${status} ${url}${body ? `: ${body}` : ''}`)
    this.status = status
    this.url = url
    this.body = body
  }
}

const joinUrl = (base, path) => {
  if (!path.startsWith('/')) path = `/${path}`
  if (!base) return path
  return `${base.replace(/\/$/, '')}${path}`
}

export async function nodeFetch(path, { signal, method = 'GET', body, headers = {} } = {}) {
  const url = joinUrl(API_BASE, path)
  const init = {
    method,
    headers: { 'Accept': 'application/json', ...headers },
    signal,
  }
  if (body !== undefined) {
    init.headers['Content-Type'] = 'application/json'
    init.body = typeof body === 'string' ? body : JSON.stringify(body)
  }
  const res = await fetch(url, init)
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    throw new ApiError(res.status, url, text.slice(0, 500))
  }
  const ct = res.headers.get('content-type') || ''
  if (ct.includes('application/json')) return res.json()
  return res.text()
}

export const nodeApi = {
  health: () => nodeFetch('/v1/health'),
  chainStats: (signal) => nodeFetch('/v1/chain/stats', { signal }),
  runtimeConfig: (signal) => nodeFetch('/v1/runtime/config', { signal }),

  blocks: ({ limit = 20, offset = 0, before, after } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    if (before != null) q.set('before_height', String(before))
    else if (after != null) q.set('after_height', String(after))
    else q.set('offset', String(offset))
    return nodeFetch(`/v1/blocks?${q.toString()}`, { signal })
  },
  blockByHeight: (height, signal) => nodeFetch(`/v1/blocks/${height}`, { signal }),
  blockByHash: (hash, signal) => nodeFetch(`/v1/blocks/hash/${hash}`, { signal }),

  transaction: (canonical, signal) => nodeFetch(`/v1/transactions/${encodeURIComponent(canonical)}`, { signal }),

  packages: ({ limit = 20, offset = 0 } = {}, signal) =>
    nodeFetch(`/v1/packages?limit=${limit}&offset=${offset}`, { signal }),
  package: (canonical, signal) =>
    nodeFetch(`/v1/packages/${encodeURIComponent(canonical)}`, { signal }),
  packageProof: (canonical, signal) =>
    nodeFetch(`/v1/packages/${encodeURIComponent(canonical)}/proof`, { signal }),

  publisher: (pubkey, signal) =>
    nodeFetch(`/v1/publishers/${encodeURIComponent(pubkey)}`, { signal }),

  pending: (signal) => nodeFetch('/v1/pending', { signal }),
  nodes: (signal) => nodeFetch('/v1/nodes', { signal }),
  p2pStatus: (signal) => nodeFetch('/v1/p2p/status', { signal }),
  bridgeStatus: (signal) => nodeFetch('/v1/bridge/status', { signal }),
  validatorRegistrations: (signal) => nodeFetch('/v1/validators/registrations', { signal }),
  validatorProfile: (address, signal) =>
    nodeFetch(`/v1/validators/${encodeURIComponent(address)}`, { signal }),
  consensusState: (signal) => nodeFetch('/v1/consensus/state', { signal }),

  addressProfile: (address, signal) =>
    nodeFetch(`/v1/addresses/${encodeURIComponent(address)}`, { signal }),
  addressTransactions: (address, { limit = 50, scan } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    if (scan != null) q.set('scan', String(scan))
    return nodeFetch(`/v1/addresses/${encodeURIComponent(address)}/transactions?${q.toString()}`, { signal })
  },
}

export { API_BASE }
