import React from 'react'
import { nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { CommitteeTelemetryPanel } from '../components/CommitteeTelemetryPanel.jsx'
import { UntrustedText } from '../components/UntrustedText.jsx'
import { formatWei } from '../utils/format.js'

export default function ValidatorList() {
  const { data, error, loading, refetch } = usePolling(
    (s) => nodeApi.validatorRegistrations(s),
    { intervalMs: 8000 },
  )
  const list = Array.isArray(data) ? data : (data?.registrations || [])
  if (error && !list.length) return <ErrorState error={error} onRetry={refetch} title="Could not load validators" />

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <header style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between' }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>Validators</h1>
        <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{list.length} total</span>
      </header>
      <CommitteeTelemetryPanel />
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
                const addr = (v.identity?.evm_address || v.evm_address || v.node_id || '').toLowerCase()
                const nodeId = v.identity?.node_id || v.node_id || '—'
                const status = v.active ? 'Active' : (v.status || v.staking_state || 'Pending')
                const stakeDisplay = v.stake != null ? `${v.stake} CREG` : '—'
                return (
                  <tr key={addr || i}>
                    <td><Hash value={addr} kind="validator" start={8} end={6} /></td>
                    <td style={{ color: 'var(--text-secondary)' }}>
                      {v.alias ? <UntrustedText value={v.alias} maxGraphemes={48} /> : '—'}
                      {v.capabilities?.ai_mode && (
                        <div style={{ fontSize: 10, color: 'var(--text-tertiary)', marginTop: 2 }}>
                          Self-reported: <UntrustedText value={v.capabilities.ai_mode} maxGraphemes={32} label="self-reported" />
                        </div>
                      )}
                    </td>
                    <td style={{ fontFamily: 'var(--font-mono)' }}>{stakeDisplay}</td>
                    <td><StatusBadge variant={status === 'Active' ? 'success' : 'warning'}>{status}</StatusBadge></td>
                    <td><Hash value={nodeId} start={6} end={4} showCopy={false} /></td>
                  </tr>
                )
              })}
          </tbody>
        </table>
      </div>
    </div>
  )
}
