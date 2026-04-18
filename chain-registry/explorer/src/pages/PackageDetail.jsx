import React from 'react'
import { useParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'

export default function PackageDetail() {
  const { id } = useParams()
  const canonical = decodeURIComponent(id)
  const { data, error, loading, refetch } = useFetch((s) => nodeApi.package(canonical, s), { deps: [canonical] })

  if (loading && !data) return <SkeletonCard lines={10} />
  if (error) return <ErrorState error={error} onRetry={refetch} title="Package not found" />
  if (!data) return <EmptyState title="Package not found" description={canonical} />

  const p = data

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header>
        <h1 style={{ margin: 0, fontSize: 18, fontFamily: 'var(--font-mono)', wordBreak: 'break-all' }}>{canonical}</h1>
        {p.revoked && <StatusBadge variant="error">Revoked</StatusBadge>}
      </header>
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <Row k="Version"   v={p.version || '—'} />
        <Row k="Publisher" v={<Hash value={p.publisher} kind="publisher" full />} />
        <Row k="IPFS cid"  v={p.ipfs_cid ? <Hash value={p.ipfs_cid} full /> : '—'} />
        <Row k="Payload hash" v={p.payload_hash ? <Hash value={p.payload_hash} full /> : '—'} />
        <Row k="Block"     v={p.block_height} />
        {p.validation && <Row k="Validation" v={<code style={{ fontSize: 11 }}>{JSON.stringify(p.validation)}</code>} />}
      </section>
      <details className="ce-card">
        <summary style={{ cursor: 'pointer', color: 'var(--text-secondary)', fontSize: 13, fontWeight: 600 }}>Raw JSON</summary>
        <pre style={{ marginTop: 'var(--space-3)', fontSize: 11, color: 'var(--text-secondary)', overflowX: 'auto' }}>{JSON.stringify(p, null, 2)}</pre>
      </details>
    </div>
  )
}

function Row({ k, v }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, wordBreak: 'break-all' }}>{v ?? '—'}</span>
    </div>
  )
}
