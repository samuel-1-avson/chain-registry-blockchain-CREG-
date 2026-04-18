import React from 'react'
import { nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { formatWei } from '../utils/format.js'

export default function ValidatorList() {
  const { data, error, loading, refetch } = usePolling(
    (s) => nodeApi.validatorRegistrations(s),
    { intervalMs: 8000 },
  )
  const list = data?.registrations || (Array.isArray(data) ? data : [])

  if (error && !list.length) return <ErrorState error={error} onRetry={refetch} title="Could not load validators" />

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <header style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between' }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>Validators</h1>
        <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{list.length} total</span>
      </header>
      <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
        <table className="ce-table">
          <thead>
            <tr>
              <th>Address</th>
              <th>Alias</th>
              <th>Stake</th>
              <th>Status</th>
              <th>Node</th>
            </tr>
          </thead>
          <tbody>
            {loading && !list.length
              ? Array.from({ length: 6 }).map((_, i) => <SkeletonRow key={i} cells={5} />)
              : list.map((v, i) => {
                const addr = (v.evm_address || v.address || '').toLowerCase()
                return (
                  <tr key={addr || i}>
                    <td><Hash value={addr} kind="validator" start={8} end={6} /></td>
                    <td style={{ color: 'var(--text-secondary)' }}>{v.alias || '—'}</td>
                    <td style={{ fontFamily: 'var(--font-mono)' }}>{formatWei(v.stake ?? v.amount)} CREG</td>
                    <td><StatusBadge variant={v.state === 'Active' ? 'success' : v.state === 'Pending' ? 'warning' : 'muted'}>{v.state || v.status || '—'}</StatusBadge></td>
                    <td><Hash value={v.node_id} start={6} end={4} showCopy={false} /></td>
                  </tr>
                )
              })}
          </tbody>
        </table>
      </div>
    </div>
  )
}
