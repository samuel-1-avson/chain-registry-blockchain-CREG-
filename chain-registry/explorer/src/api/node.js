// Chain Registry node API client.
// Single source of truth for /v1/* calls made by the explorer.
// Every helper returns parsed JSON or throws ApiError with the status code.

const API_BASE = import.meta.env.VITE_API_BASE || ''
const ENDPOINT_UNAVAILABLE_STATUSES = new Set([404, 405, 501])

export class ApiError extends Error {
  constructor(status, url, body) {
    super(`${status} ${url}${body ? `: ${body}` : ''}`)
    this.status = status
    this.url = url
    this.body = body
  }
}

function attachEndpointStatus(payload, status) {
  if (payload && typeof payload === 'object') {
    payload.__endpointStatus = status
    return payload
  }

  return { value: payload, __endpointStatus: status }
}

function isEndpointUnavailable(error) {
  return error instanceof ApiError && ENDPOINT_UNAVAILABLE_STATUSES.has(error.status)
}

function optionalNodeFetch(path, fallbackValue, status, options = {}) {
  return nodeFetch(path, options).catch((error) => {
    if (!isEndpointUnavailable(error)) throw error
    const fallback = typeof fallbackValue === 'function' ? fallbackValue() : fallbackValue
    return attachEndpointStatus(fallback, {
      kind: 'endpoint-unavailable',
      path,
      statusCode: error.status,
      ...status,
    })
  })
}

export function getEndpointStatus(payload) {
  return payload && typeof payload === 'object' ? payload.__endpointStatus || null : null
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
    return optionalNodeFetch(
      `/v1/addresses/${encodeURIComponent(address)}/stakes?${q.toString()}`,
      () => ({ stakes: [], total: 0 }),
      {
        feature: 'Address stake history',
        message: 'This node does not expose historical address stake data yet.',
      },
      { signal },
    )
  },

  validatorBlocks: (address, { limit = 25 } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    return optionalNodeFetch(
      `/v1/validators/${encodeURIComponent(address)}/blocks?${q.toString()}`,
      () => ({ blocks: [], total: 0 }),
      {
        feature: 'Validator block history',
        message: 'This node does not expose validator block history yet.',
      },
      { signal },
    )
  },
  validatorVotes: (address, { limit = 50 } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    return optionalNodeFetch(
      `/v1/validators/${encodeURIComponent(address)}/votes?${q.toString()}`,
      () => ({ votes: [], total: 0 }),
      {
        feature: 'Validator vote history',
        message: 'This node does not expose validator vote history yet.',
      },
      { signal },
    )
  },

  /** Smart search — tries /v1/search first, falls back to client-side lookup. */
  search: (query, signal) =>
    optionalNodeFetch(
      `/v1/search?q=${encodeURIComponent(query)}`,
      () => ({ matches: [] }),
      {
        feature: 'Search index',
        message: 'Full-text search is unavailable on this node. The explorer is falling back to direct block, address, validator, and package lookups.',
      },
      { signal },
    ),

  /** Bridge anchor history — graceful fallback if endpoint not deployed. */
  bridgeAnchors: (signal) =>
    optionalNodeFetch(
      '/v1/bridge/anchors',
      () => ({ anchors: [] }),
      {
        feature: 'Bridge anchor history',
        message: 'This node is serving bridge status, but it does not expose historical bridge anchor data yet.',
      },
      { signal },
    ),

  /** Governance proposals — graceful fallback if endpoint not deployed. */
  governanceProposals: (signal) =>
    optionalNodeFetch(
      '/v1/governance/proposals',
      () => ({ proposals: [] }),
      {
        feature: 'Governance proposals',
        message: 'This node does not expose governance proposal data yet.',
      },
      { signal },
    ),

  /** Metrics time-series — graceful fallback if endpoint not deployed. */
  metricsHistory: ({ range = '1h' } = {}, signal) =>
    optionalNodeFetch(
      `/v1/metrics/history?range=${range}`,
      () => ({ samples: [] }),
      {
        feature: 'Metrics history',
        message: 'This node does not expose historical metrics samples yet.',
      },
      { signal },
    ),

  /** Chain Reorganizations. */
  reorgs: (signal) =>
    optionalNodeFetch(
      '/v1/reorgs',
      () => [],
      {
        feature: 'Reorg history',
        message: 'This node does not expose chain reorganization history yet.',
      },
      { signal },
    ),

  /** Rich List of top accounts. */
  richList: (signal) =>
    optionalNodeFetch(
      '/v1/richlist',
      () => [],
      {
        feature: 'Rich list',
        message: 'This node does not expose rich-list data yet.',
      },
      { signal },
    ),
}

export { API_BASE }
