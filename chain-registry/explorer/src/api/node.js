// Chain Registry node API client.
// Prefer grouped route prefixes and only fall back to legacy aliases when the
// node clearly does not implement the grouped endpoint yet.

const API_BASE = import.meta.env.VITE_API_BASE || ''
const OPERATOR_API_KEY = import.meta.env.VITE_OPERATOR_API_KEY || ''
const LEGACY_FALLBACK_STATUSES = new Set([404, 405, 501])
const OPTIONAL_ENDPOINT_STATUSES = new Set([401, 403, 404, 405, 501, 503])

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

function isLegacyFallbackEligible(error) {
  return error instanceof ApiError && LEGACY_FALLBACK_STATUSES.has(error.status)
}

function isOptionalEndpointUnavailable(error) {
  return error instanceof ApiError && OPTIONAL_ENDPOINT_STATUSES.has(error.status)
}

function withScopeHeaders(scope, headers = {}) {
  if (scope !== 'operator' || !OPERATOR_API_KEY) return headers
  return { 'X-Operator-Key': OPERATOR_API_KEY, ...headers }
}

async function scopedNodeFetch(path, { scope = 'public', headers = {}, ...options } = {}) {
  return nodeFetch(path, {
    ...options,
    headers: withScopeHeaders(scope, headers),
  })
}

async function groupedNodeFetch(groupedPath, legacyPath, options = {}) {
  try {
    return await scopedNodeFetch(groupedPath, options)
  } catch (error) {
    if (!legacyPath || !isLegacyFallbackEligible(error)) throw error
    return scopedNodeFetch(legacyPath, options)
  }
}

function optionalGroupedNodeFetch(groupedPath, legacyPath, fallbackValue, status, options = {}) {
  return groupedNodeFetch(groupedPath, legacyPath, options).catch((error) => {
    if (!isOptionalEndpointUnavailable(error)) throw error
    const fallback = typeof fallbackValue === 'function' ? fallbackValue() : fallbackValue
    return attachEndpointStatus(fallback, {
      kind: 'endpoint-unavailable',
      path: groupedPath,
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
  health: () => groupedNodeFetch('/v1/public/health', '/v1/health'),
  chainStats: (signal) => groupedNodeFetch('/v1/public/chain/stats', '/v1/chain/stats', { signal }),
  runtimeConfig: (signal) => optionalGroupedNodeFetch(
    '/v1/operator/runtime/config',
    '/v1/runtime/config',
    () => ({}),
    {
      feature: 'Runtime configuration',
      message: 'Runtime configuration requires operator credentials on this node.',
    },
    { signal, scope: 'operator' },
  ),

  blocks: ({ limit = 20, offset = 0, before, after } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    if (before != null) q.set('before_height', String(before))
    else if (after != null) q.set('after_height', String(after))
    else q.set('offset', String(offset))
    return groupedNodeFetch(`/v1/public/blocks?${q.toString()}`, `/v1/blocks?${q.toString()}`, { signal })
  },
  blockByHeight: (height, signal) => groupedNodeFetch(`/v1/public/blocks/${height}`, `/v1/blocks/${height}`, { signal }),
  blockByHash: (hash, signal) => groupedNodeFetch(`/v1/public/blocks/hash/${hash}`, `/v1/blocks/hash/${hash}`, { signal }),

  transaction: (canonical, signal) => groupedNodeFetch(
    `/v1/public/transactions/${encodeURIComponent(canonical)}`,
    `/v1/transactions/${encodeURIComponent(canonical)}`,
    { signal },
  ),

  packages: ({ limit = 20, offset = 0 } = {}, signal) =>
    groupedNodeFetch(`/v1/public/packages?limit=${limit}&offset=${offset}`, `/v1/packages?limit=${limit}&offset=${offset}`, { signal }),
  package: (canonical, signal) =>
    groupedNodeFetch(`/v1/public/packages/${encodeURIComponent(canonical)}`, `/v1/packages/${encodeURIComponent(canonical)}`, { signal }),
  packageProof: (canonical, signal) =>
    groupedNodeFetch(`/v1/public/packages/${encodeURIComponent(canonical)}/proof`, `/v1/packages/${encodeURIComponent(canonical)}/proof`, { signal }),

  publisher: (pubkey, signal) =>
    groupedNodeFetch(`/v1/public/publishers/${encodeURIComponent(pubkey)}`, `/v1/publishers/${encodeURIComponent(pubkey)}`, { signal }),

  pending: (signal) => optionalGroupedNodeFetch(
    '/v1/operator/pending',
    '/v1/pending',
    () => ({ count: 0, packages: [] }),
    {
      feature: 'Pending pool',
      message: 'Pending pool data requires operator credentials on this node.',
    },
    { signal, scope: 'operator' },
  ),
  nodes: (signal) => optionalGroupedNodeFetch(
    '/v1/operator/nodes',
    '/v1/nodes',
    () => ({ nodes: [] }),
    {
      feature: 'Node list',
      message: 'Network topology requires operator credentials on this node.',
    },
    { signal, scope: 'operator' },
  ),
  p2pStatus: (signal) => optionalGroupedNodeFetch(
    '/v1/operator/p2p/status',
    '/v1/p2p/status',
    () => ({}),
    {
      feature: 'P2P status',
      message: 'Peer transport details require operator credentials on this node.',
    },
    { signal, scope: 'operator' },
  ),
  bridgeStatus: (signal) => groupedNodeFetch('/v1/public/bridge/status', '/v1/bridge/status', { signal }),
  validatorRegistrations: (signal) => groupedNodeFetch('/v1/validator/registrations', '/v1/validators/registrations', { signal }),
  validatorProfile: (address, signal) =>
    groupedNodeFetch(`/v1/public/validators/${encodeURIComponent(address)}`, `/v1/validators/${encodeURIComponent(address)}`, { signal }),
  consensusState: (signal) => groupedNodeFetch('/v1/validator/consensus/state', '/v1/consensus/state', { signal }),

  addressProfile: (address, signal) =>
    groupedNodeFetch(`/v1/public/addresses/${encodeURIComponent(address)}`, `/v1/addresses/${encodeURIComponent(address)}`, { signal }),
  addressTransactions: (address, { limit = 50, scan } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    if (scan != null) q.set('scan', String(scan))
    return groupedNodeFetch(
      `/v1/public/addresses/${encodeURIComponent(address)}/transactions?${q.toString()}`,
      `/v1/addresses/${encodeURIComponent(address)}/transactions?${q.toString()}`,
      { signal },
    )
  },
  addressStakes: (address, { limit = 50 } = {}, signal) => {
    const q = new URLSearchParams()
    q.set('limit', String(limit))
    return optionalGroupedNodeFetch(
      `/v1/public/addresses/${encodeURIComponent(address)}/stakes?${q.toString()}`,
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
    return optionalGroupedNodeFetch(
      `/v1/public/validators/${encodeURIComponent(address)}/blocks?${q.toString()}`,
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
    return optionalGroupedNodeFetch(
      `/v1/public/validators/${encodeURIComponent(address)}/votes?${q.toString()}`,
      `/v1/validators/${encodeURIComponent(address)}/votes?${q.toString()}`,
      () => ({ votes: [], total: 0 }),
      {
        feature: 'Validator vote history',
        message: 'This node does not expose validator vote history yet.',
      },
      { signal },
    )
  },

  /** Smart search — tries grouped search first, then falls back to the legacy alias. */
  search: (query, signal) =>
    optionalGroupedNodeFetch(
      `/v1/public/search?q=${encodeURIComponent(query)}`,
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
    optionalGroupedNodeFetch(
      '/v1/public/bridge/anchors',
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
    optionalGroupedNodeFetch(
      '/v1/public/governance/proposals',
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
    optionalGroupedNodeFetch(
      `/v1/operator/metrics/history?range=${range}`,
      `/v1/metrics/history?range=${range}`,
      () => ({ samples: [] }),
      {
        feature: 'Metrics history',
        message: 'Historical metrics require operator credentials on this node.',
      },
      { signal, scope: 'operator' },
    ),

  /** Chain Reorganizations. */
  reorgs: (signal) =>
    optionalGroupedNodeFetch(
      '/v1/public/reorgs',
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
    optionalGroupedNodeFetch(
      '/v1/public/richlist',
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
