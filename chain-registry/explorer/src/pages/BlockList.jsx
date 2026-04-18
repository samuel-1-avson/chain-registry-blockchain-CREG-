import React, { useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { Pagination } from '../components/Pagination.jsx'
import { SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState } from '../components/ErrorState.jsx'
import { formatNumber } from '../utils/format.js'

const PAGE_SIZE = 25

export default function BlockList() {
  const [params, setParams] = useSearchParams()
  const page = Math.max(0, parseInt(params.get('page') || '0', 10) || 0)
  const pageSize = PAGE_SIZE

  const { data, error, loading, refetch } = usePolling(
    (signal) => nodeApi.blocks({ limit: pageSize, offset: page * pageSize }, signal),
    { intervalMs: 8000, deps: [page] },
  )

  const blocks = Array.isArray(data) ? data : (data?.blocks || [])
  const total = (data?.total ?? blocks.length + page * pageSize)

  const goto = (p) => {
    params.set('page', String(p))
    setParams(params, { replace: true })
  }

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <header style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 16 }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>Blocks</h1>
        <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>
          {loading && !blocks.length ? 'loading…' : `showing ${blocks.length} of ${formatNumber(total)}`}
        </span>
      </header>

      {error && !blocks.length ? (
        <ErrorState error={error} onRetry={refetch} title="Could not load blocks" />
      ) : (
        <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
          <table className="ce-table">
            <thead>
              <tr>
                <th style={{ width: 100 }}>Height</th>
                <th>Hash</th>
                <th style={{ width: 80 }}>Txs</th>
                <th style={{ width: 180 }}>Producer</th>
                <th style={{ width: 140 }}>Age</th>
              </tr>
            </thead>
            <tbody>
              {loading && !blocks.length
                ? Array.from({ length: 10 }).map((_, i) => <SkeletonRow key={i} cells={5} />)
                : blocks.map((b) => (
                  <tr key={b.height ?? b.hash}>
                    <td style={{ fontFamily: 'var(--font-mono)', fontWeight: 600 }}>
                      <Link to={`/block/${b.height}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>#{b.height}</Link>
                    </td>
                    <td><Hash value={b.hash} kind="block-hash" start={8} end={6} /></td>
                    <td style={{ color: 'var(--text-secondary)' }}>{b.tx_count ?? b.transactions?.length ?? 0}</td>
                    <td><Hash value={b.producer} kind="validator" start={6} end={4} /></td>
                    <td><TimeAgo timestamp={b.timestamp_ms ?? b.timestamp} /></td>
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
      )}

      <Pagination page={page} pageSize={pageSize} total={total} onPage={goto} />
    </div>
  )
}
