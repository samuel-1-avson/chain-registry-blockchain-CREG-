// Pure untrusted-text sanitization & URL validation helpers (ADR-019 ECO-INV-05, eco-adv-11).

const CONTROL_AND_BIDI_RE =
  /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u200E\u200F\u061C\u202A-\u202E\u2066-\u2069]/g

export function sanitizeUntrustedText(value) {
  if (value == null) return ''
  return String(value).replace(/\r\n?/g, '\n').replace(CONTROL_AND_BIDI_RE, '')
}

export function truncateGraphemes(text, maxGraphemes = 128) {
  const cap = Number.isInteger(maxGraphemes) && maxGraphemes > 0 ? maxGraphemes : 128
  const SegmenterCtor = typeof Intl !== 'undefined' ? Intl.Segmenter : undefined
  const units = SegmenterCtor
    ? Array.from(new SegmenterCtor(undefined, { granularity: 'grapheme' }).segment(text), (s) => s.segment)
    : Array.from(text)
  if (units.length <= cap) {
    return { text, truncated: false, totalGraphemes: units.length }
  }
  return {
    text: `${units.slice(0, cap).join('')}...`,
    truncated: true,
    totalGraphemes: units.length,
  }
}

export function safeL1TxUrl(txHash, explorerOrigin = 'https://sepolia.etherscan.io') {
  if (typeof txHash !== 'string' || !/^0x[0-9a-fA-F]{64}$/.test(txHash.trim())) {
    return null
  }
  try {
    const origin = new URL(explorerOrigin)
    if (origin.protocol !== 'https:') return null
    return `${origin.origin}/tx/${txHash.trim().toLowerCase()}`
  } catch {
    return null
  }
}
