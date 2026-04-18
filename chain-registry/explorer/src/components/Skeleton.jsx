import React from 'react'

export function Skeleton({ width = '100%', height = 16, radius = 6, style = {} }) {
  return (
    <span
      aria-busy="true"
      style={{
        display: 'inline-block',
        width,
        height,
        borderRadius: radius,
        background: 'linear-gradient(90deg, var(--surface) 0%, var(--surface-hover) 50%, var(--surface) 100%)',
        backgroundSize: '200% 100%',
        animation: 'chain-skeleton-pulse 1.4s ease-in-out infinite',
        ...style,
      }}
    />
  )
}

export function SkeletonRow({ cells = 4 }) {
  return (
    <tr>
      {Array.from({ length: cells }).map((_, i) => (
        <td key={i} style={{ padding: 'var(--space-3)' }}><Skeleton height={14} /></td>
      ))}
    </tr>
  )
}

export function SkeletonCard({ lines = 4 }) {
  return (
    <div style={{
      padding: 'var(--space-6)',
      background: 'var(--surface)',
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius-lg)',
      display: 'grid',
      gap: 'var(--space-3)',
    }}>
      <Skeleton width="40%" height={18} />
      {Array.from({ length: lines }).map((_, i) => (
        <Skeleton key={i} width={`${60 + (i * 7) % 35}%`} height={12} />
      ))}
    </div>
  )
}
