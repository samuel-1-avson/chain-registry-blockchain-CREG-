import React from 'react'
import { nodeApi } from '../api/node.js'
import { useChainStats } from '../hooks/useStats.js'
import { usePolling } from '../hooks/usePolling.js'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'

export default function Network() {
  const consensus = usePolling((s) => nodeApi.consensusState(s), { intervalMs: 10_000 })
  const chain = useChainStats(10_000)

  if (consensus.loading && !consensus.data) return <SkeletonCard lines={6} />

  const list = consensus.data?.validators || []
  const onlineValidators = list.filter((validator) => validator.status === 'online' || validator.status === 'active').length

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <h1 style={{ margin: 0, fontSize: 20 }}>Network</h1>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))', gap: 'var(--space-4)' }}>
        <div className="ce-stat"><span className="ce-stat-label">Connected peers</span><span className="ce-stat-value">{chain.data?.peer_count ?? '—'}</span></div>
        <div className="ce-stat"><span className="ce-stat-label">Online validators</span><span className="ce-stat-value">{consensus.data ? `${onlineValidators} / ${consensus.data.total_validators ?? list.length}` : '—'}</span></div>
        <div className="ce-stat"><span className="ce-stat-label">PBFT quorum</span><span className="ce-stat-value">{consensus.data?.quorum ?? '—'}</span></div>
      </div>

      {consensus.error ? (
        <ErrorState error={consensus.error} onRetry={consensus.refetch} title="Could not load validator set" />
      ) : list.length === 0 ? (
        <EmptyState title="No validators reported" description="The public consensus endpoint did not report a validator roster." />
      ) : (
        <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
          <table className="ce-table">
            <thead><tr><th>Validator</th><th>Stake</th><th>Reputation</th><th>Status</th></tr></thead>
            <tbody>
              {list.map((validator, i) => {
                const online = validator.status === 'online' || validator.status === 'active'
                return (
                <tr key={validator.id || i}>
                  <td style={{ color: 'var(--text-primary)', fontFamily: 'var(--font-mono)' }}>{validator.alias || validator.id || '—'}</td>
                  <td>{validator.stake ?? '—'} CREG</td>
                  <td>{validator.reputation ?? '—'}</td>
                  <td><StatusBadge variant={online ? 'success' : 'error'}>{validator.status || 'unknown'}</StatusBadge></td>
                </tr>
              )})}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
