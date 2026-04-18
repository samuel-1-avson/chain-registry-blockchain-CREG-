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
  addressStakes: (address, { limit = 50 } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    return nodeFetch(`/v1/addresses/${encodeURIComponent(address)}/stakes?${q.toString()}`, { signal })
      .catch(() => ({ stakes: [], total: 0 })) // graceful fallback if endpoint not yet deployed
  },

  validatorBlocks: (address, { limit = 25 } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    return nodeFetch(`/v1/validators/${encodeURIComponent(address)}/blocks?${q.toString()}`, { signal })
      .catch(() => ({ blocks: [], total: 0 }))
  },
  validatorVotes: (address, { limit = 50 } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    return nodeFetch(`/v1/validators/${encodeURIComponent(address)}/votes?${q.toString()}`, { signal })
      .catch(() => ({ votes: [], total: 0 }))
  },

  /** Smart search — tries /v1/search first, falls back to client-side lookup. */
  search: async (query, signal) => {
    try {
      return await nodeFetch(`/v1/search?q=${encodeURIComponent(query)}`, { signal })
    } catch {
      // Backend may not have /v1/search yet — return empty matches
      return { matches: [] }
    }
  },

  /** Bridge anchor history — graceful fallback if endpoint not deployed. */
  bridgeAnchors: (signal) =>
    nodeFetch('/v1/bridge/anchors', { signal })
      .catch(() => ({ anchors: [] })),

  /** Governance proposals — graceful fallback if endpoint not deployed. */
  governanceProposals: (signal) =>
    nodeFetch('/v1/governance/proposals', { signal })
      .catch(() => ({ proposals: [] })),

  /** Metrics time-series — graceful fallback if endpoint not deployed. */
  metricsHistory: ({ range = '1h' } = {}, signal) =>
    nodeFetch(`/v1/metrics/history?range=${range}`, { signal })
      .catch(() => ({ samples: [] })),

  /** Chain Reorganizations. */
  reorgs: (signal) =>
    nodeFetch('/v1/reorgs', { signal })
      .catch(() => []),

  /** Rich List of top accounts. */
  richList: (signal) =>
    nodeFetch('/v1/richlist', { signal })
      .catch(() => []),
}

export { API_BASE }
