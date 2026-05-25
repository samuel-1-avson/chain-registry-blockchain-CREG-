// Shared formatting helpers used across pages.
// Pull these from a single place so every view formats amounts and hashes identically.

/** Convert a decimal wei string (e.g. "1000000000000000000") to a human display ("1.0000"). */
export function formatWei(wei, decimals = 18, precision = 4) {
  if (wei == null) return '—'
  const s = String(wei)
  if (!/^\d+$/.test(s)) return s
  if (s === '0') return '0'
  if (s.length <= decimals) {
    const pad = '0'.repeat(decimals - s.length) + s
    const trimmed = pad.replace(/0+$/, '')
    return `0.${trimmed.slice(0, precision).padEnd(Math.min(precision, trimmed.length), '0')}`
  }
  const whole = s.slice(0, s.length - decimals)
  const frac = s.slice(s.length - decimals).replace(/0+$/, '')
  if (!frac) return addThousandsSep(whole)
  return `${addThousandsSep(whole)}.${frac.slice(0, precision)}`
}

export function addThousandsSep(n) {
  return String(n).replace(/\B(?=(\d{3})+(?!\d))/g, ',')
}

export function truncateHash(hash, start = 8, end = 8) {
  if (!hash || typeof hash !== 'string') return hash ?? ''
  if (hash.length <= start + end + 1) return hash
  return `${hash.slice(0, start)}…${hash.slice(-end)}`
}

/** Lower-cased canonical EVM address check: 0x + 40 hex. */
export function isEvmAddress(s) {
  if (typeof s !== 'string') return false
  const trimmed = s.trim()
  return /^0x[a-fA-F0-9]{40}$/.test(trimmed)
}

/** 32-byte hex hash, with or without a 0x prefix. */
export function isHash32(s) {
  if (typeof s !== 'string') return false
  return /^(0x)?[a-fA-F0-9]{64}$/.test(s.trim())
}

/** Integer height check (accepts decimal strings). */
export function isBlockHeight(s) {
  if (typeof s !== 'string') return false
  return /^\d+$/.test(s.trim())
}

/** Package canonical: `<name>@<version>` or scoped variants. Lightweight check. */
export function isPackageCanonical(s) {
  return typeof s === 'string' && s.includes('@') && !s.startsWith('0x')
}

export function formatNumber(num) {
  if (num == null || isNaN(num)) return '—'
  const n = Number(num)
  if (n >= 1e9) return (n / 1e9).toFixed(2) + 'B'
  if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M'
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'k'
  return addThousandsSep(Math.round(n))
}

export function timeAgo(timestamp) {
  if (timestamp == null) return '—'
  const t = typeof timestamp === 'number' ? timestamp : Date.parse(timestamp)
  if (!Number.isFinite(t)) return '—'
  const seconds = Math.floor((Date.now() - t) / 1000)
  if (seconds < 0) return 'just now'
  if (seconds < 60) return `${seconds}s ago`
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`
  return `${Math.floor(seconds / 86400)}d ago`
}

/** Classify a free-text search query for the global search bar. */
export function classifySearch(raw) {
  const q = (raw || '').trim()
  if (!q) return { kind: 'empty' }
  if (isBlockHeight(q)) return { kind: 'block-height', value: q }
  if (isEvmAddress(q)) return { kind: 'address', value: q.toLowerCase() }
  if (isHash32(q)) return { kind: 'hash', value: q.toLowerCase() }
  if (isPackageCanonical(q)) return { kind: 'package', value: q }
  return { kind: 'text', value: q }
}
