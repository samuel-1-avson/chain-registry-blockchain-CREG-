import React from 'react'
import { nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'

export default function Bridge() {
  const { data, error, loading, refetch } = usePolling((s) => nodeApi.bridgeStatus(s), { intervalMs: 10_000 })
  if (loading && !data) return <SkeletonCard lines={8} />
  if (error) return <ErrorState error={error} onRetry={refetch} title="Could not load bridge status" />

  const s = data || {}
  const health = s.healthy ?? (s.last_anchor_block != null ? 'healthy' : 'unknown')

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>L1 bridge</h1>
        <StatusBadge variant={health === 'healthy' || health === true ? 'success' : 'warning'}>{String(health)}</StatusBadge>
      </header>
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <Row k="L1 chain"         v={s.l1_chain_id ?? s.chain_id ?? '—'} />
        <Row k="Bridge contract"  v={s.bridge_contract ? <Hash value={s.bridge_contract} full /> : '—'} />
        <Row k="Last anchor block" v={s.last_anchor_block ?? s.last_committed ?? '—'} />
        <Row k="Last anchor root"  v={s.last_anchor_root ? <Hash value={s.last_anchor_root} full /> : '—'} />
        <Row k="Signer"            v={s.signer_address ? <Hash value={s.signer_address} full /> : '—'} />
        <Row k="Commit cadence"    v={s.commit_interval ?? '—'} />
      </section>
      <section className="ce-card">
        <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 14 }}>Anchor log</h2>
        <p style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>
          Anchor history is populated from <code style={{ fontSize: 11 }}>GET /v1/bridge/anchors</code> (Sprint 3).
          This view shows the latest committed state from <code style={{ fontSize: 11 }}>/v1/bridge/status</code> for now.
        </p>
      </section>
      {!s.last_anchor_block && <EmptyState title="No anchors yet" description="The bridge hasn't committed any state to L1 on this network." />}
    </div>
  )
}

function Row({ k, v }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '180px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, wordBreak: 'break-all' }}>{v}</span>
    </div>
  )
}
