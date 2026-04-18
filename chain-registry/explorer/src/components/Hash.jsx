import React from 'react'
import { Link } from 'react-router-dom'
import { truncateHash } from '../utils/format.js'
import { CopyButton } from './CopyButton.jsx'

/**
 * Render a hash with truncation, optional link, and copy button.
 * `to` — explicit href (takes precedence)
 * `kind` — 'block' | 'tx' | 'address' | 'package' | 'publisher'
 */
export function Hash({ value, to, kind, start = 8, end = 8, mono = true, showCopy = true, full = false }) {
  if (!value) return <span style={{ color: 'var(--text-tertiary)' }}>—</span>
  const display = full ? value : truncateHash(value, start, end)
  let href = to
  if (!href && kind) {
    const encoded = encodeURIComponent(value)
    switch (kind) {
      case 'block': href = `/block/${value}`; break
      case 'block-hash': href = `/block/hash/${value}`; break
      case 'tx': href = `/tx/${encoded}`; break
      case 'address': href = `/address/${value}`; break
      case 'validator': href = `/validator/${value}`; break
      case 'package': href = `/package/${encoded}`; break
      case 'publisher': href = `/publisher/${encoded}`; break
      default: href = null
    }
  }
  const content = (
    <span
      style={{
        fontFamily: mono ? 'var(--font-mono)' : 'inherit',
        color: href ? 'var(--accent-primary-light)' : 'var(--text-primary)',
        fontSize: '12px',
      }}
      title={value}
    >
      {display}
    </span>
  )
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 'var(--space-2)' }}>
      {href ? <Link to={href} style={{ textDecoration: 'none' }}>{content}</Link> : content}
      {showCopy && <CopyButton value={value} compact size="xs" title={`Copy ${kind || 'value'}`} />}
    </span>
  )
}
