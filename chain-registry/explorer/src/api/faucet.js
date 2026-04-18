// Faucet API client. Matches chain-registry/crates/faucet routes.

const DEFAULT_FAUCET_BASE = import.meta.env.VITE_FAUCET_URL || ''

const joinUrl = (base, path) => {
  if (!path.startsWith('/')) path = `/${path}`
  if (!base) return path
  return `${base.replace(/\/$/, '')}${path}`
}

async function faucetFetch(base, path, init) {
  const url = joinUrl(base, path)
  const res = await fetch(url, init)
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    throw new Error(`faucet ${res.status} ${path}${text ? `: ${text.slice(0, 200)}` : ''}`)
  }
  return res.json()
}

export const faucetApi = {
  network: (base = DEFAULT_FAUCET_BASE) => faucetFetch(base, '/api/network'),
  challenge: (base = DEFAULT_FAUCET_BASE) => faucetFetch(base, '/api/challenge'),
  balance: (address, base = DEFAULT_FAUCET_BASE) => {
    const hex = address.startsWith('0x') ? address.slice(2) : address
    return faucetFetch(base, `/api/balance/${hex}`)
  },
  drip: (payload, base = DEFAULT_FAUCET_BASE) =>
    faucetFetch(base, '/api/drip', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    }),
}

export { DEFAULT_FAUCET_BASE }
