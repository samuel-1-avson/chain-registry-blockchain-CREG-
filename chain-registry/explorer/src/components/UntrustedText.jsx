import React, { useState } from 'react'
import { sanitizeUntrustedText, truncateGraphemes } from '../utils/untrustedText.js'

export function UntrustedText({
  value,
  maxGraphemes = 128,
  expandable = false,
  label = 'untrusted',
}) {
  const [expanded, setExpanded] = useState(false)
  const sanitized = sanitizeUntrustedText(value)
  const { text, truncated } = truncateGraphemes(sanitized, maxGraphemes)
  const shown = expanded ? sanitized : text

  return (
    <span style={{ display: 'inline-flex', flexWrap: 'wrap', alignItems: 'baseline', gap: 6 }}>
      <span
        data-untrusted="true"
        title={truncated ? `Untrusted content (truncated to ${maxGraphemes} graphemes)` : 'Untrusted content'}
        style={{
          unicodeBidi: 'isolate',
          whiteSpace: 'pre-wrap',
          fontFamily: 'var(--font-mono)',
          wordBreak: 'break-word',
          color: 'var(--text-primary)',
        }}
      >
        {shown}
      </span>
      <span
        style={{
          fontSize: 10,
          lineHeight: 1.2,
          padding: '1px 5px',
          borderRadius: 4,
          border: '1px solid var(--border)',
          color: 'var(--text-tertiary)',
          userSelect: 'none',
        }}
      >
        {label}
      </span>
      {expandable && truncated && (
        <button
          type="button"
          onClick={() => setExpanded((prev) => !prev)}
          style={{
            background: 'transparent',
            border: 'none',
            padding: 0,
            color: 'var(--accent-primary-light)',
            fontSize: 11,
            cursor: 'pointer',
            textDecoration: 'underline',
          }}
        >
          {expanded ? 'Show less' : 'Show full'}
        </button>
      )}
    </span>
  )
}
