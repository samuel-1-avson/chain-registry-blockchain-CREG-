import React from 'react'
import { Link, useLocation } from 'react-router-dom'

export default function NotFound() {
  const loc = useLocation()
  return (
    <div className="ce-card" style={{ padding: 'var(--space-12) var(--space-6)', textAlign: 'center', display: 'grid', gap: 'var(--space-4)' }}>
      <h1 style={{ margin: 0, fontSize: 48, fontFamily: 'var(--font-mono)', color: 'var(--accent-primary-light)' }}>404</h1>
      <p style={{ color: 'var(--text-secondary)', margin: 0 }}>No route matches <code>{loc.pathname}</code>.</p>
      <div>
        <Link to="/" style={{ color: 'var(--accent-primary-light)', textDecoration: 'none', fontSize: 13 }}>← Back to dashboard</Link>
      </div>
    </div>
  )
}
