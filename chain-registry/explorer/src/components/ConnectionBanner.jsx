import React from 'react'
import { SSE_STATE } from '../hooks/useSse.js'

const MESSAGES = {
  [SSE_STATE.Idle]: null,
  [SSE_STATE.Connecting]: { variant: 'info', label: 'Connecting live feed…' },
  [SSE_STATE.Live]: null,
  [SSE_STATE.Stale]: { variant: 'warning', label: 'Live feed idle — no events for 30 s' },
  [SSE_STATE.Error]: { variant: 'error', label: 'Live feed offline — attempting to reconnect' },
}

const COLORS = {
  info: { bg: 'rgba(59,130,246,0.12)', fg: 'var(--accent-info)', border: 'rgba(59,130,246,0.3)' },
  warning: { bg: 'rgba(245,158,11,0.12)', fg: 'var(--accent-warning)', border: 'rgba(245,158,11,0.3)' },
  error: { bg: 'rgba(239,68,68,0.12)', fg: 'var(--accent-error)', border: 'rgba(239,68,68,0.3)' },
}

/**
 * Small pill shown in the header when the SSE feed is connecting, stale, or offline.
 * Hidden when the feed is live. Lets users know whether the data they see is fresh.
 */
export function ConnectionBanner({ state, reconnectAttempt }) {
  const msg = MESSAGES[state]
  if (!msg) return null
  const c = COLORS[msg.variant]
  return (
    <span role="status" aria-live="polite" style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: '6px',
      padding: '4px 10px',
      borderRadius: 'var(--radius-full)',
      background: c.bg,
      color: c.fg,
      border: `1px solid ${c.border}`,
      fontSize: 11,
      fontWeight: 600,
      letterSpacing: '0.02em',
      textTransform: 'uppercase',
    }}>
      <span style={{ width: 6, height: 6, borderRadius: '50%', background: c.fg }} />
      {msg.label}
      {reconnectAttempt > 0 && state === SSE_STATE.Error ? ` (try ${reconnectAttempt})` : ''}
    </span>
  )
}
