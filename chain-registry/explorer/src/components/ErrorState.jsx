import React from 'react'

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
