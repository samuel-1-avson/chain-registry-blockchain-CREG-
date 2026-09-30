// Chain Registry node API client.
// Prefer grouped route prefixes and only fall back to legacy aliases when the
// node clearly does not implement the grouped endpoint yet.

import { normalizeStats, normalizeBlock, normalizeBlocks, normalizeTransaction } from './normalize.js'

const API_BASE = import.meta.env.VITE_API_BASE || ''
const NODE_ROUTE_MODE = (import.meta.env.VITE_NODE_ROUTE_MODE || 'auto').toLowerCase()
const LEGACY_FALLBACK_STATUSES = new Set([404, 405, 501])
const OPTIONAL_ENDPOINT_STATUSES = new Set([401, 403, 404, 405, 501, 503])
const legacyEndpoints = new Set()
// During public-services bootstrap every CREG API route intentionally returns
// 503 until an independently operated observer is attached. Cache that
// explicit state briefly so a dozen polling hooks do not hammer the edge and
// fill the browser console with identical failures. A later probe automatically
// resumes once the observer becomes available.
const OBSERVER_RETRY_MS = 30_000
let observerUnavailableUntil = 0

export function isObserverUnavailable() {
  return Date.now() < observerUnavailableUntil
}

function markObserverUnavailable(error) {
  if (error instanceof ApiError && error.status === 503 && /observer_unavailable/i.test(error.body || '')) {
    observerUnavailableUntil = Date.now() + OBSERVER_RETRY_MS
  }
}

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

function shouldPreferLegacyRoutes(path) {
  return NODE_ROUTE_MODE === 'legacy' || legacyEndpoints.has(path.split('?')[0])
}

function isOptionalEndpointUnavailable(error) {
  return error instanceof ApiError && OPTIONAL_ENDPOINT_STATUSES.has(error.status)
}

async function scopedNodeFetch(path, options = {}) {
  // A browser bundle must never contain the operator service credential.
  // Operator-scoped reads therefore fail closed and use their existing
  // endpoint-unavailable fallbacks unless a server-side trusted proxy is used.
  const requestOptions = { ...options }
  delete requestOptions.scope
  return nodeFetch(path, requestOptions)
}

async function groupedNodeFetch(groupedPath, legacyPath, options = {}) {
  if (legacyPath && shouldPreferLegacyRoutes(groupedPath)) {
    return scopedNodeFetch(legacyPath, options)
  }

  if (NODE_ROUTE_MODE === 'grouped') {
    return scopedNodeFetch(groupedPath, options)
  }

  try {
    return await scopedNodeFetch(groupedPath, options)
  } catch (error) {
    if (!legacyPath || !isLegacyFallbackEligible(error)) throw error
    legacyEndpoints.add(groupedPath.split('?')[0])
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
  if (isObserverUnavailable() && path.startsWith('/v1/')) {
    throw new ApiError(503, url, '{"error":"observer_unavailable","message":"The validator observer pool is not attached yet.","cached":true}')
  }
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
    const error = new ApiError(res.status, url, text.slice(0, 500))
    markObserverUnavailable(error)
    throw error
  }
  const ct = res.headers.get('content-type') || ''
  if (ct.includes('application/json')) return res.json()
  return res.text()
}

export const nodeApi = {
  health: () => groupedNodeFetch('/v1/public/health', '/v1/health'),
  chainStats: (signal) => groupedNodeFetch('/v1/public/chain/stats', '/v1/chain/stats', { signal }).then(normalizeStats),
  runtimeConfig: async (signal) => {
    const [health, stats] = await Promise.all([
      groupedNodeFetch('/v1/public/health', '/v1/health', { signal }),
      groupedNodeFetch('/v1/public/chain/stats', '/v1/chain/stats', { signal }),
    ])
    return { ...health, ...stats }
  },
  operatorRuntimeConfig: (signal) => optionalGroupedNodeFetch(
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
    return groupedNodeFetch(`/v1/public/blocks?${q.toString()}`, `/v1/blocks?${q.toString()}`, { signal }).then(async (payload) => {
      // Older observers omit genesis from the list while serving /blocks/0.
      if (payload.tip_height === 0 && payload.blocks?.length === 0 && offset === 0 && before == null && after == null && limit > 0) {
        return { ...payload, blocks: [await nodeApi.blockByHeight(0, signal)] }
      }
      return normalizeBlocks(payload)
    })
  },
  blockByHeight: (height, signal) => groupedNodeFetch(`/v1/public/blocks/${height}`, `/v1/blocks/${height}`, { signal }).then(normalizeBlock),
  blockByHash: (hash, signal) => groupedNodeFetch(`/v1/public/blocks/hash/${hash}`, `/v1/blocks/hash/${hash}`, { signal }).then(normalizeBlock),

  transaction: (canonical, signal) => groupedNodeFetch(
    `/v1/public/transactions/${encodeURIComponent(canonical)}`,
    `/v1/transactions/${encodeURIComponent(canonical)}`,
    { signal },
  ).then((payload) => ({ ...payload, transaction: normalizeTransaction(payload.transaction || payload) })),

  packages: ({ limit = 20, offset = 0 } = {}, signal) =>
    groupedNodeFetch(`/v1/public/packages?limit=${limit}&offset=${offset}`, `/v1/packages?limit=${limit}&offset=${offset}`, { signal }).then(payload => {
      const project = item => ({ ...item, timestamp: item.published_at ?? item.timestamp })
      return Array.isArray(payload) ? payload.map(project) : { ...payload, packages: (payload.packages || []).map(project) }
    }),
  package: (canonical, signal) =>
    groupedNodeFetch(`/v1/public/packages/${encodeURIComponent(canonical)}`, `/v1/packages/${encodeURIComponent(canonical)}`, { signal }),
  packageProof: (canonical, signal) =>
    groupedNodeFetch(`/v1/public/packages/${encodeURIComponent(canonical)}/proof`, `/v1/packages/${encodeURIComponent(canonical)}/proof`, { signal }),
  packageIntelligence: (canonical, signal) =>
    groupedNodeFetch(
      `/v1/public/packages/${encodeURIComponent(canonical)}/intelligence`,
      `/v1/packages/${encodeURIComponent(canonical)}/intelligence`,
      { signal },
    ),

  publisher: (pubkey, signal) =>
    groupedNodeFetch(`/v1/public/publishers/${encodeURIComponent(pubkey)}`, `/v1/publishers/${encodeURIComponent(pubkey)}`, { signal }),

  pending: (signal) => optionalGroupedNodeFetch(
    '/v1/operator/pending',
    '/v1/pending',
    () => ({ count: 0, packages: [] }),
    {
      feature: 'Pending pool',
      message: 'The count above comes from public chain statistics. Individual pending items require operator credentials.',
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
  validatorRegistrations: async (signal) => {
    const [registrations, consensus] = await Promise.all([
      groupedNodeFetch('/v1/validator/registrations', '/v1/validators/registrations', { signal }),
      groupedNodeFetch('/v1/validator/consensus/state', '/v1/consensus/state', { signal }),
    ])
    const list = Array.isArray(registrations) ? registrations : (registrations.registrations || [])
    const active = (consensus.validators || []).map(v => {
      const registration = list.find(r => (r.identity?.node_id || r.node_id) === v.id)
      return { ...registration, ...v, node_id: v.id, active: true }
    })
    return [...active, ...list.filter(r => !active.some(v => v.node_id === (r.identity?.node_id || r.node_id)))]
  },
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

  /** Durable public metrics history from the node's Prometheus collector. */
  metricsHistory: ({ range = '1h' } = {}, signal) =>
    optionalGroupedNodeFetch(
      `/v1/public/metrics/history?range=${encodeURIComponent(range)}`,
      null,
      () => ({ status: 'unavailable', samples: [] }),
      {
        feature: 'Metrics history',
        message: 'This node does not provide historical metrics through the public API. The collector and history route must be connected to the same chain before charts can appear.',
      },
      { signal },
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
  richList: async (signal) => {
    const data = await optionalGroupedNodeFetch(
      '/v1/public/richlist',
      '/v1/richlist',
      () => [],
      {
        feature: 'Rich list',
        message: 'This node does not expose rich-list data yet.',
      },
      { signal },
    )
    if (Array.isArray(data) && data.length === 0 && !getEndpointStatus(data)) {
      const consensus = await nodeApi.consensusState(signal)
      return (consensus.validators || []).map(v => ({ ...v, active: true,
        identity: { node_id: v.id }, stake: v.stake }))
    }
    return data
  },
}

export { API_BASE }
export { NODE_ROUTE_MODE }
