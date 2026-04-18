import React from 'react'

const btn = (active, disabled) => ({
  padding: '6px 12px',
  minWidth: 36,
  borderRadius: 'var(--radius-sm)',
  border: `1px solid ${active ? 'var(--border-accent)' : 'var(--border)'}`,
  background: active ? 'rgba(99,102,241,0.12)' : 'var(--surface)',
  color: disabled ? 'var(--text-disabled)' : active ? 'var(--accent-primary-light)' : 'var(--text-secondary)',
  cursor: disabled ? 'not-allowed' : 'pointer',
  fontSize: '12px',
  fontFamily: 'var(--font-sans)',
  transition: 'all var(--transition-fast)',
})

/** Offset-based pagination. For cursor-based lists use <CursorPager>. */
export function Pagination({ page, pageSize, total, onPage }) {
  const pages = Math.max(1, Math.ceil((total || 0) / pageSize))
  if (pages <= 1) return null
  const canPrev = page > 0
  const canNext = page < pages - 1
  return (
    <nav aria-label="Pagination" style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)', justifyContent: 'flex-end', padding: 'var(--space-3) 0' }}>
      <button style={btn(false, !canPrev)} disabled={!canPrev} onClick={() => onPage(0)}>«</button>
      <button style={btn(false, !canPrev)} disabled={!canPrev} onClick={() => onPage(page - 1)}>‹</button>
      <span style={{ color: 'var(--text-secondary)', fontSize: '12px', padding: '0 var(--space-2)' }}>
        Page <strong style={{ color: 'var(--text-primary)' }}>{page + 1}</strong> of {pages}
      </span>
      <button style={btn(false, !canNext)} disabled={!canNext} onClick={() => onPage(page + 1)}>›</button>
      <button style={btn(false, !canNext)} disabled={!canNext} onClick={() => onPage(pages - 1)}>»</button>
    </nav>
  )
}

/** Cursor pager — callers pass `before`/`after` handlers. Useful for live lists. */
export function CursorPager({ canNewer = false, canOlder = false, onNewer, onOlder, label = 'items' }) {
  return (
    <nav aria-label="Cursor pagination" style={{ display: 'flex', gap: 'var(--space-2)', justifyContent: 'flex-end', padding: 'var(--space-3) 0' }}>
      <button style={btn(false, !canNewer)} disabled={!canNewer} onClick={onNewer}>← Newer {label}</button>
      <button style={btn(false, !canOlder)} disabled={!canOlder} onClick={onOlder}>Older {label} →</button>
    </nav>
  )
}
