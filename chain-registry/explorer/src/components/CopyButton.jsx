import React from 'react'
import { useClipboard } from '../hooks/useClipboard.js'

export function CopyButton({ value, label = 'Copy', size = 'sm', compact = false, title }) {
  const { copied, copy } = useClipboard()
  const onClick = (e) => {
    e.preventDefault()
    e.stopPropagation()
    copy(value)
  }
  const style = {
    background: copied ? 'rgba(34, 197, 94, 0.14)' : 'var(--surface)',
    color: copied ? 'var(--accent-success)' : 'var(--text-secondary)',
    border: `1px solid ${copied ? 'rgba(34,197,94,0.35)' : 'var(--border)'}`,
    borderRadius: 'var(--radius-sm)',
    padding: size === 'xs' ? '2px 6px' : '4px 8px',
    fontSize: size === 'xs' ? '10px' : '11px',
    cursor: 'pointer',
    transition: 'all var(--transition-fast)',
    fontFamily: 'var(--font-sans)',
  }
  return (
    <button type="button" onClick={onClick} style={style} aria-label={title || 'Copy to clipboard'} title={title || 'Copy'}>
      {copied ? '✓ Copied' : (compact ? '⧉' : label)}
    </button>
  )
}
