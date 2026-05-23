import React, { useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { Pagination } from '../components/Pagination.jsx'
import { SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { formatNumber } from '../utils/format.js'

const PAGE_SIZE = 20

export default function PackageList() {
  const [params, setParams] = useSearchParams()
  const page = Math.max(0, parseInt(params.get('page') || '0', 10) || 0)
  const { data, error, loading, refetch } = usePolling(
    (s) => nodeApi.packages({ limit: PAGE_SIZE, offset: page * PAGE_SIZE }, s),
    { intervalMs: 10_000, deps: [page] },
  )
  const items = data?.packages || (Array.isArray(data) ? data : [])
  const total = data?.total ?? items.length + page * PAGE_SIZE

  if (error && !items.length) return <ErrorState error={error} onRetry={refetch} title="Could not load packages" />
  if (!loading && !items.length) return <EmptyState title="No packages yet" description="Once publishers start shipping, their canonical hashes will appear here." />

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <header style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between' }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>Packages</h1>
        <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{formatNumber(total)} total</span>
      </header>
      <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
        <table className="ce-table">
          <thead>
            <tr>
              <th>Canonical</th>
              <th>Version</th>
              <th>Publisher</th>
              <th>Age</th>
            </tr>
          </thead>
          <tbody>
            {loading && !items.length
              ? Array.from({ length: 8 }).map((_, i) => <SkeletonRow key={i} cells={4} />)
              : items.map((p) => (
                <tr key={p.canonical}>
                  <td>
                    <Link to={`/package/${encodeURIComponent(p.canonical)}`} style={{ color: 'var(--accent-primary-light)', fontFamily: 'var(--font-mono)', textDecoration: 'none' }}>
                      {p.canonical}
                    </Link>
                  </td>
                  <td style={{ color: 'var(--text-secondary)', fontFamily: 'var(--font-mono)' }}>{p.version || '—'}</td>
                  <td><Hash value={p.publisher} kind="publisher" start={6} end={4} /></td>
                  <td><TimeAgo timestamp={p.timestamp} /></td>
                </tr>
              ))}
          </tbody>
        </table>
      </div>
      <Pagination page={page} pageSize={PAGE_SIZE} total={total} onPage={(p) => { params.set('page', String(p)); setParams(params, { replace: true }) }} />
    </div>
  )
}
