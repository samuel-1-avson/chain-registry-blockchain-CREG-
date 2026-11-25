import React, { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { findingCategoryLabel } from '../api/normalize.js'
import { usePolling } from '../hooks/usePolling.js'
import { SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { UntrustedText } from '../components/UntrustedText.jsx'

function formatRemaining(seconds) {
  if (seconds <= 0) return 'Expired — failing closed to REVOKED'
  const hrs = Math.floor(seconds / 3600)
  const mins = Math.floor((seconds % 3600) / 60)
  const secs = Math.floor(seconds % 60)
  return `${hrs}h ${mins}m ${secs}s`
}

export default function QuarantineQueue({ sseState = 'connecting', sseTick = 0 }) {
  const [fetchedPerfMs, setFetchedPerfMs] = useState(0)
  const [elapsedSecs, setElapsedSecs] = useState(0)

  const query = usePolling(
    async (signal) => {
      const res = await nodeApi.quarantineQueue({ limit: 100 }, signal)
      setFetchedPerfMs(typeof performance !== 'undefined' ? performance.now() : 0)
      setElapsedSecs(0)
      return res
    },
    { intervalMs: 30_000, enabled: sseState !== 'live' },
  )

  useEffect(() => {
    if (sseTick > 0) query.refetch()
  }, [sseTick])

  useEffect(() => {
    if (!query.data) return
    const timer = setInterval(() => {
      const nowPerf = typeof performance !== 'undefined' ? performance.now() : 0
      setElapsedSecs(Math.max(0, Math.floor((nowPerf - fetchedPerfMs) / 1000)))
    }, 1000)
    return () => clearInterval(timer)
  }, [query.data, fetchedPerfMs])

  const entries = query.data?.entries || []
  const serverNow = Number(query.data?.server_time_unix ?? 0) + elapsedSecs

  if (query.error && !entries.length) {
    return <ErrorState error={query.error} onRetry={query.refetch} title="Could not load quarantine queue" />
  }

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', flexWrap: 'wrap', gap: 12 }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>72h Quarantine Escalation Queue</h1>
        <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{entries.length} active holds</span>
      </header>

      <div
        role="alert"
        className="ce-card"
        style={{
          borderColor: 'var(--accent-warning)',
          background: 'rgba(245, 158, 11, 0.08)',
          color: 'var(--accent-warning)',
          fontSize: 13,
          fontWeight: 600,
        }}
      >
        Quarantined packages cannot be installed and have no break-glass override. Fails closed to REVOKED at expiry unless released by committee quorum.
      </div>

      {query.loading && !query.data ? (
        <div className="ce-card">
          <table className="ce-table">
            <tbody>
              <SkeletonRow cells={5} />
              <SkeletonRow cells={5} />
            </tbody>
          </table>
        </div>
      ) : entries.length === 0 ? (
        <EmptyState
          title="No active quarantine holds"
          description="Packages placed under Class C review appear here sorted earliest-deadline first."
        />
      ) : (
        <div className="ce-card" style={{ padding: 0, overflowX: 'auto' }}>
          <table className="ce-table">
            <thead>
              <tr>
                <th>Package</th>
                <th>Category</th>
                <th>Opened</th>
                <th>Remaining (Server Clock)</th>
                <th>Committee</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry, idx) => {
                const remaining = serverNow > 0 ? entry.expires_at_unix - serverNow : 0
                const c = entry.committee || {}
                return (
                  <tr key={`${entry.canonical}-${idx}`}>
                    <td>
                      <Link to={`/package/${encodeURIComponent(entry.canonical)}`}>
                        <UntrustedText value={entry.canonical} maxGraphemes={96} />
                      </Link>
                    </td>
                    <td>
                      <StatusBadge variant="warning">{findingCategoryLabel(entry.reason_code)}</StatusBadge>
                    </td>
                    <td>
                      {entry.opened_height != null ? (
                        <Link to={`/block/${entry.opened_height}`}>#{entry.opened_height}</Link>
                      ) : '—'}
                    </td>
                    <td style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
                      {serverNow > 0 ? formatRemaining(remaining) : new Date(entry.expires_at_unix * 1000).toISOString()}
                    </td>
                    <td style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                      <StatusBadge variant="success">+{c.approvals ?? 0}</StatusBadge>
                      <StatusBadge variant="warning">?{c.quarantine_reviews ?? 0}</StatusBadge>
                      <StatusBadge variant="error">-{c.rejections ?? 0}</StatusBadge>
                      <StatusBadge variant="muted">H:{c.hybrid_signers ?? 0}/C:{c.classical_signers ?? 0}</StatusBadge>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
