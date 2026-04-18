import React from 'react'
import { Link } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'

export default function Pending() {
  const { data, error, loading, refetch } = usePolling((s) => nodeApi.pending(s), { intervalMs: 3000 })
  const items = data?.pending || data?.items || (Array.isArray(data) ? data : [])

  if (error && !items.length) return <ErrorState error={error} onRetry={refetch} title="Could not load mempool" />

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <header style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between' }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>Pending pool</h1>
        <StatusBadge variant="info" pulse>{items.length} waiting</StatusBadge>
      </header>
      {!loading && !items.length ? (
        <EmptyState title="Mempool is empty" description="No pending transactions — the chain is caught up." />
      ) : (
        <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
          <table className="ce-table">
            <thead>
              <tr>
                <th>Canonical</th>
                <th>Publisher</th>
                <th>Received</th>
                <th>Stage</th>
              </tr>
            </thead>
            <tbody>
              {loading && !items.length
                ? Array.from({ length: 5 }).map((_, i) => <SkeletonRow key={i} cells={4} />)
                : items.map((t, i) => (
                  <tr key={t.canonical || t.id || i}>
                    <td>
                      {t.canonical ? (
                        <Link to={`/tx/${encodeURIComponent(t.canonical)}`} style={{ color: 'var(--accent-primary-light)', fontFamily: 'var(--font-mono)', textDecoration: 'none' }}>{t.canonical}</Link>
                      ) : (
                        <Hash value={t.hash || t.id} kind="tx" />
                      )}
                    </td>
                    <td><Hash value={t.publisher} kind="publisher" start={6} end={4} /></td>
                    <td><TimeAgo timestamp={t.received_at || t.timestamp_ms || t.timestamp} /></td>
                    <td><StatusBadge variant="warning">{t.stage || 'pending'}</StatusBadge></td>
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
