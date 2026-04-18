import React from 'react'

const VARIANTS = {
  success: { fg: 'var(--accent-success)', bg: 'rgba(34, 197, 94, 0.15)', border: 'rgba(34, 197, 94, 0.3)' },
  warning: { fg: 'var(--accent-warning)', bg: 'rgba(245, 158, 11, 0.15)', border: 'rgba(245, 158, 11, 0.3)' },
  error:   { fg: 'var(--accent-error)',   bg: 'rgba(239, 68, 68, 0.15)',  border: 'rgba(239, 68, 68, 0.3)' },
  info:    { fg: 'var(--accent-info)',    bg: 'rgba(59, 130, 246, 0.15)', border: 'rgba(59, 130, 246, 0.3)' },
  muted:   { fg: 'var(--text-secondary)', bg: 'var(--surface)',           border: 'var(--border)' },
}

export function StatusBadge({ variant = 'muted', children, icon, pulse = false }) {
  const v = VARIANTS[variant] || VARIANTS.muted
  return (
    <span style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: '6px',
      padding: '3px 10px',
      borderRadius: 'var(--radius-full)',
      background: v.bg,
      color: v.fg,
      border: `1px solid ${v.border}`,
      fontSize: '11px',
      fontWeight: 600,
      letterSpacing: '0.02em',
      textTransform: 'uppercase',
    }}>
      {pulse && (
        <span style={{
          width: 6, height: 6, borderRadius: '50%', background: v.fg,
          boxShadow: `0 0 0 0 ${v.fg}`,
          animation: 'chain-pulse 1.5s ease-in-out infinite',
        }} />
      )}
      {icon && <span aria-hidden="true">{icon}</span>}
      {children}
    </span>
  )
}
