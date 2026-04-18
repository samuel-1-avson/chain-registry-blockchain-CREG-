import React from 'react'
import { nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'

export default function Network() {
  const peers = usePolling((s) => nodeApi.nodes(s), { intervalMs: 10_000 })
  const p2p = usePolling((s) => nodeApi.p2pStatus(s), { intervalMs: 10_000 })

  if (peers.loading && !peers.data) return <SkeletonCard lines={6} />

  const list = peers.data?.nodes || (Array.isArray(peers.data) ? peers.data : [])
  const stats = p2p.data || {}

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <h1 style={{ margin: 0, fontSize: 20 }}>Network</h1>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))', gap: 'var(--space-4)' }}>
        <div className="ce-stat"><span className="ce-stat-label">Peers</span><span className="ce-stat-value">{list.length}</span></div>
        <div className="ce-stat"><span className="ce-stat-label">Listen addr</span><span className="ce-stat-value" style={{ fontSize: 14 }}>{stats.listen_addr || '—'}</span></div>
        <div className="ce-stat"><span className="ce-stat-label">Pubkey</span><span className="ce-stat-value" style={{ fontSize: 12 }}>{stats.pubkey ? stats.pubkey.slice(0, 18) + '…' : '—'}</span></div>
      </div>

      {peers.error ? (
        <ErrorState error={peers.error} onRetry={peers.refetch} title="Could not load peer list" />
      ) : list.length === 0 ? (
        <EmptyState title="No peers connected" description="This node is running solo. Start another node and peer them over libp2p." />
      ) : (
        <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
          <table className="ce-table">
            <thead><tr><th>Node ID</th><th>Address</th><th>Role</th><th>Status</th></tr></thead>
            <tbody>
              {list.map((n, i) => (
                <tr key={n.id || n.node_id || i}>
                  <td><Hash value={n.id || n.node_id} start={10} end={6} /></td>
                  <td style={{ color: 'var(--text-secondary)', fontFamily: 'var(--font-mono)', fontSize: 11 }}>{n.address || n.multiaddr || '—'}</td>
                  <td style={{ color: 'var(--text-secondary)' }}>{n.role || '—'}</td>
                  <td><StatusBadge variant={n.connected !== false ? 'success' : 'error'}>{n.connected !== false ? 'connected' : 'offline'}</StatusBadge></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
