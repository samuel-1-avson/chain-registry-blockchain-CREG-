import React, { useCallback, useState } from 'react'

/**
 * Reusable "Share" button — copies the current page URL to clipboard.
 * Shows a brief "Copied!" toast inline before resetting.
 */
export function ShareButton({ url, label = 'Share', size = 'sm' }) {
  const [copied, setCopied] = useState(false)

  const onClick = useCallback(() => {
    const target = url || window.location.href
    navigator.clipboard.writeText(target).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    }).catch(() => {
      // Fallback for HTTP contexts
      const ta = document.createElement('textarea')
      ta.value = target
      ta.style.position = 'fixed'
      ta.style.left = '-9999px'
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      document.body.removeChild(ta)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    })
  }, [url])

  const isSmall = size === 'sm'

  return (
    <button
      type="button"
      onClick={onClick}
      title="Copy link to clipboard"
      aria-label={copied ? 'Copied!' : label}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: isSmall ? '5px 10px' : '8px 14px',
        background: copied ? 'rgba(34,197,94,0.12)' : 'var(--surface)',
        border: `1px solid ${copied ? 'var(--accent-success)' : 'var(--border)'}`,
        borderRadius: 'var(--radius-sm)',
        color: copied ? 'var(--accent-success)' : 'var(--text-secondary)',
        fontSize: isSmall ? 11 : 12,
        fontFamily: 'var(--font-sans)',
        fontWeight: 600,
        cursor: 'pointer',
        transition: 'all var(--transition-fast)',
        whiteSpace: 'nowrap',
      }}
    >
      <span aria-hidden="true">{copied ? '✓' : '🔗'}</span>
      {copied ? 'Copied!' : label}
    </button>
  )
}
