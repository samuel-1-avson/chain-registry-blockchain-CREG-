// Sponsored-stake relayer client.
// The relayer forwards pre-signed stake intents so users can stake without gas.

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
  status: (base = DEFAULT_RELAYER_BASE) => relayerFetch(base, '/v1/relayer/status'),
  policy: (base = DEFAULT_RELAYER_BASE) => relayerFetch(base, '/v1/relayer/policy'),
  quota: (owner, base = DEFAULT_RELAYER_BASE) =>
    relayerFetch(base, `/v1/relayer/quota/${owner}`),
  submit: (payload, base = DEFAULT_RELAYER_BASE) =>
    relayerFetch(base, '/v1/relayer/stake', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    }),
}

export { DEFAULT_RELAYER_BASE }
