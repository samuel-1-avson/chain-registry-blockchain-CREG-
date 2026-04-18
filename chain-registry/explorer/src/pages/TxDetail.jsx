import React from 'react'
import { useParams, Link } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'

/** /tx/:canonical — canonical may be `name@version` or a 0x hash. */
export default function TxDetail() {
  const { id } = useParams()
  const canonical = decodeURIComponent(id)
  const { data, error, loading, refetch } = useFetch(
    (signal) => nodeApi.transaction(canonical, signal),
    { deps: [canonical] },
  )

  if (loading && !data) return <SkeletonCard lines={10} />
  if (error) return <ErrorState error={error} onRetry={refetch} title="Transaction not found" />
  if (!data) return <EmptyState title="Transaction not found" description={canonical} />

  const t = data
  const status = t.status || (t.included ? 'included' : 'pending')

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
        <h1 style={{ margin: 0, fontSize: 18, fontFamily: 'var(--font-mono)', wordBreak: 'break-all' }}>{canonical}</h1>
        <StatusBadge variant={status === 'finalized' ? 'success' : status === 'revoked' ? 'error' : 'info'}>{status}</StatusBadge>
      </header>

      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <Row k="Canonical"  v={t.canonical || canonical} mono />
        <Row k="Version"    v={t.version} />
        <Row k="Publisher"  v={<Hash value={t.publisher} kind="publisher" full />} />
        <Row k="Block"      v={t.block_height != null ? <Link to={`/block/${t.block_height}`} style={{ color: 'var(--accent-primary-light)' }}>#{t.block_height}</Link> : 'pending'} />
        <Row k="Included at" v={t.included_at ? <TimeAgo timestamp={t.included_at} /> : '—'} />
        <Row k="IPFS cid"   v={t.ipfs_cid ? <Hash value={t.ipfs_cid} full /> : '—'} />
        <Row k="Payload hash" v={t.payload_hash ? <Hash value={t.payload_hash} full /> : '—'} />
      </section>

      {t.validation && (
        <section>
          <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 15 }}>Validation report</h2>
          <div className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
            {Object.entries(t.validation).map(([k, v]) => (
              <Row key={k} k={k.replace(/_/g, ' ')} v={formatValidationValue(v)} />
            ))}
          </div>
        </section>
      )}

      <details className="ce-card">
        <summary style={{ cursor: 'pointer', color: 'var(--text-secondary)', fontSize: 13, fontWeight: 600 }}>Raw JSON</summary>
        <pre style={{ marginTop: 'var(--space-3)', fontSize: 11, color: 'var(--text-secondary)', overflowX: 'auto', background: 'var(--bg-elevated)', padding: 'var(--space-3)', borderRadius: 'var(--radius-sm)' }}>
          {JSON.stringify(t, null, 2)}
        </pre>
      </details>
    </div>
  )
}

function formatValidationValue(v) {
  if (v == null) return '—'
  if (typeof v === 'object') return <code style={{ fontSize: 11 }}>{JSON.stringify(v)}</code>
  if (typeof v === 'boolean') return <StatusBadge variant={v ? 'success' : 'error'}>{v ? 'pass' : 'fail'}</StatusBadge>
  return String(v)
}

function Row({ k, v, mono }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, fontFamily: mono ? 'var(--font-mono)' : 'inherit', wordBreak: 'break-all' }}>{v ?? '—'}</span>
    </div>
  )
}
