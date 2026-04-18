import React from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { isHash32 } from '../utils/format.js'

/** /block/:id — id is either a decimal height or a 0x… hash. */
export default function BlockDetail() {
  const { id } = useParams()
  const nav = useNavigate()

  const byHash = isHash32(id)
  const { data, error, loading, refetch } = useFetch(
    (signal) => (byHash ? nodeApi.blockByHash(id, signal) : nodeApi.blockByHeight(id, signal)),
    { deps: [id] },
  )

  if (loading && !data) return <SkeletonCard lines={10} />
  if (error) return <ErrorState error={error} onRetry={refetch} title={`Block ${id} not found`} />
  if (!data) return <EmptyState title="Block not found" description={`No block matches "${id}".`} />

  const b = data
  const height = b.height ?? b.number
  const timestamp = b.timestamp_ms ?? b.timestamp
  const txs = b.transactions || b.txs || []
  const votes = b.votes || b.approvals || []

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', flexWrap: 'wrap' }}>
        <h1 style={{ margin: 0, fontSize: 22, fontFamily: 'var(--font-mono)', letterSpacing: '-0.02em' }}>Block #{height}</h1>
        <StatusBadge variant={b.finalized ? 'success' : 'warning'} pulse={!b.finalized}>
          {b.finalized ? 'Finalized' : (b.phase || 'Pending')}
        </StatusBadge>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
          <button type="button" onClick={() => nav(`/block/${Math.max(0, Number(height) - 1)}`)}
            style={navBtn} disabled={!height || height === 0}>← Prev</button>
          <button type="button" onClick={() => nav(`/block/${Number(height) + 1}`)} style={navBtn}>Next →</button>
        </div>
      </header>

      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <Row k="Hash"         v={<Hash value={b.hash} full showCopy />} />
        <Row k="Previous"     v={<Hash value={b.prev_hash || b.previous_hash} kind="block-hash" full />} />
        <Row k="Timestamp"    v={<>{timestamp} <span style={{ color: 'var(--text-tertiary)', marginLeft: 8 }}><TimeAgo timestamp={timestamp} /></span></>} />
        <Row k="Producer"     v={<Hash value={b.producer} kind="validator" full />} />
        <Row k="Tx count"     v={txs.length} />
        <Row k="Size"         v={b.size_bytes ? `${b.size_bytes.toLocaleString()} B` : '—'} />
        <Row k="Votes / quorum" v={`${votes.length}${b.quorum ? ` / ${b.quorum}` : ''}`} />
        {b.signature && <Row k="Signature" v={<Hash value={b.signature} full showCopy />} />}
      </section>

      {txs.length > 0 && (
        <section>
          <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 15 }}>Transactions</h2>
          <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
            <table className="ce-table">
              <thead>
                <tr>
                  <th style={{ width: 60 }}>#</th>
                  <th>Canonical</th>
                  <th>Publisher</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {txs.map((t, i) => (
                  <tr key={t.canonical || t.hash || i}>
                    <td style={{ color: 'var(--text-tertiary)', fontFamily: 'var(--font-mono)' }}>{i}</td>
                    <td>
                      {t.canonical ? (
                        <Link to={`/tx/${encodeURIComponent(t.canonical)}`} style={{ color: 'var(--accent-primary-light)', fontFamily: 'var(--font-mono)', textDecoration: 'none' }}>
                          {t.canonical}
                        </Link>
                      ) : (
                        <Hash value={t.hash} kind="tx" full />
                      )}
                    </td>
                    <td><Hash value={t.publisher} kind="publisher" start={6} end={4} /></td>
                    <td><StatusBadge variant={t.status === 'finalized' ? 'success' : 'info'}>{t.status || 'included'}</StatusBadge></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      <details className="ce-card">
        <summary style={{ cursor: 'pointer', color: 'var(--text-secondary)', fontSize: 13, fontWeight: 600 }}>Raw JSON</summary>
        <pre style={{ marginTop: 'var(--space-3)', fontSize: 11, color: 'var(--text-secondary)', overflowX: 'auto', background: 'var(--bg-elevated)', padding: 'var(--space-3)', borderRadius: 'var(--radius-sm)' }}>
          {JSON.stringify(b, null, 2)}
        </pre>
      </details>
    </div>
  )
}

const navBtn = {
  padding: '6px 12px',
  background: 'var(--surface)',
  border: '1px solid var(--border)',
  borderRadius: 'var(--radius-sm)',
  color: 'var(--text-secondary)',
  fontSize: 12,
  cursor: 'pointer',
}

function Row({ k, v }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>{v ?? '—'}</span>
    </div>
  )
}
