// Sponsored-stake relayer client (matches crates/relayer routes).

const DEFAULT_RELAYER_BASE = import.meta.env.VITE_RELAYER_URL || ''

const joinUrl = (base, path) => {
  if (!path.startsWith('/')) path = `/${path}`
  if (!base) return path
  return `${base.replace(/\/$/, '')}${path}`
}

async function relayerFetch(base, path, init = {}) {
  const url = joinUrl(base, path)
  const res = await fetch(url, init)
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    throw new Error(`relayer ${res.status} ${path}${text ? `: ${text.slice(0, 200)}` : ''}`)
  }
  return res.json()
}

export const relayerApi = {
  policy: (base = DEFAULT_RELAYER_BASE) => relayerFetch(base, '/v1/relayer/policy'),
  quote: (payload, base = DEFAULT_RELAYER_BASE) =>
    relayerFetch(base, '/v1/relayer/quote', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    }),
  sponsor: (payload, base = DEFAULT_RELAYER_BASE) =>
    relayerFetch(base, '/v1/relayer/sponsor', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    }),
  status: (requestId, base = DEFAULT_RELAYER_BASE) =>
    relayerFetch(base, `/v1/relayer/status/${encodeURIComponent(requestId)}`),
  health: (base = DEFAULT_RELAYER_BASE) => relayerFetch(base, '/health'),
}

export { DEFAULT_RELAYER_BASE }
