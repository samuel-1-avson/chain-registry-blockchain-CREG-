import React from 'react'

const NOTICE_COLORS = {
  info: { bg: 'rgba(59,130,246,0.08)', border: 'rgba(59,130,246,0.22)', fg: 'var(--accent-info)' },
  warning: { bg: 'rgba(245,158,11,0.08)', border: 'rgba(245,158,11,0.22)', fg: 'var(--accent-warning)' },
  error: { bg: 'rgba(239,68,68,0.08)', border: 'rgba(239,68,68,0.22)', fg: 'var(--accent-error)' },
}

export function ErrorState({ error, onRetry, title = 'Something went wrong' }) {
  const message = error?.message || String(error || 'Unknown error')
  return (
    <div role="alert" style={{
      padding: 'var(--space-6)',
      background: 'rgba(239,68,68,0.06)',
      border: '1px solid rgba(239,68,68,0.2)',
      borderRadius: 'var(--radius-lg)',
      color: 'var(--text-primary)',
      display: 'grid',
      gap: 'var(--space-3)',
      maxWidth: 720,
    }}>
      <h3 style={{ margin: 0, fontSize: 16, color: 'var(--accent-error)' }}>{title}</h3>
      <code style={{ fontSize: 12, color: 'var(--text-secondary)', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
        {message}
      </code>
      {onRetry && (
        <div>
          <button type="button" onClick={onRetry} style={{
            padding: '8px 14px',
            background: 'var(--accent-primary)',
            color: '#fff',
            border: 'none',
            borderRadius: 'var(--radius-sm)',
            cursor: 'pointer',
            fontSize: 12,
            fontWeight: 600,
          }}>Retry</button>
        </div>
      )}
    </div>
  )
}

export function EmptyState({ title = 'Nothing here yet', description, action }) {
  return (
    <div style={{
      padding: 'var(--space-10) var(--space-6)',
      textAlign: 'center',
      color: 'var(--text-secondary)',
      border: '1px dashed var(--border)',
      borderRadius: 'var(--radius-lg)',
    }}>
      <div style={{ fontSize: 14, color: 'var(--text-primary)', fontWeight: 600, marginBottom: 'var(--space-2)' }}>{title}</div>
      {description && <p style={{ margin: '0 auto', maxWidth: 520, fontSize: 13 }}>{description}</p>}
      {action && <div style={{ marginTop: 'var(--space-4)' }}>{action}</div>}
    </div>
  )
}

export function NoticeState({ title = 'Notice', description, variant = 'warning' }) {
  const colors = NOTICE_COLORS[variant] || NOTICE_COLORS.warning
  return (
    <div role="status" style={{
      padding: 'var(--space-4)',
      background: colors.bg,
      border: `1px solid ${colors.border}`,
      borderLeft: `3px solid ${colors.fg}`,
      borderRadius: 'var(--radius-lg)',
      color: 'var(--text-primary)',
      display: 'grid',
      gap: 'var(--space-2)',
    }}>
      <h3 style={{ margin: 0, fontSize: 14, color: colors.fg }}>{title}</h3>
      {description && (
        <div style={{ fontSize: 12, color: 'var(--text-secondary)', lineHeight: 1.6 }}>
          {description}
        </div>
      )}
    </div>
  )
}

export function EndpointStatusNotice({ status, title }) {
  if (!status) return null

  const heading = title || (status.kind === 'endpoint-unavailable'
    ? `${status.feature || 'Endpoint'} unavailable`
    : `${status.feature || 'Endpoint'} degraded`)

  return (
    <NoticeState
      title={heading}
      variant="warning"
      description={(
        <>
          <div>{status.message || 'This explorer panel is running in a degraded mode.'}</div>
          {status.path && (
            <div>
              Endpoint: <code style={{ fontSize: 11 }}>{status.path}</code>
              {status.statusCode ? ` (${status.statusCode})` : ''}
            </div>
          )}
        </>
      )}
    />
  )
}
