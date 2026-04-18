import React, { useCallback, useEffect } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useChainStats } from '../hooks/useStats.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ShareButton } from '../components/ShareButton.jsx'
import { isHash32, formatNumber } from '../utils/format.js'

/** /block/:id — id is either a decimal height or a 0x… hash. */
export default function BlockDetail() {
  const { id } = useParams()
  const nav = useNavigate()
  const stats = useChainStats(8000)
  const tipHeight = stats.data?.current_height

  const byHash = isHash32(id)
  const { data, error, loading, refetch } = useFetch(
    (signal) => (byHash ? nodeApi.blockByHash(id, signal) : nodeApi.blockByHeight(id, signal)),
    { deps: [id] },
  )

  // Keyboard shortcuts: ← prev, → next
  const height = data?.height ?? data?.number
  const canPrev = height != null && height > 0
  const canNext = height != null && (tipHeight == null || height < tipHeight)

  const goPrev = useCallback(() => {
    if (canPrev) nav(`/block/${Math.max(0, Number(height) - 1)}`)
  }, [canPrev, height, nav])
  const goNext = useCallback(() => {
    if (canNext) nav(`/block/${Number(height) + 1}`)
  }, [canNext, height, nav])

  useEffect(() => {
    const onKey = (e) => {
      if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return
      if (e.key === 'ArrowLeft') goPrev()
      if (e.key === 'ArrowRight') goNext()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [goPrev, goNext])

  if (loading && !data) return <SkeletonCard lines={10} />
  if (error) return <ErrorState error={error} onRetry={refetch} title={`Block ${id} not found`} />
  if (!data) return <EmptyState title="Block not found" description={`No block matches "${id}".`} />

  const b = data
  const timestamp = b.timestamp_ms ?? b.timestamp ?? b.header?.timestamp
  const txs = b.transactions || b.txs || []
  const votes = b.votes || b.approvals || []
  const isFinalized = b.finalized ?? false
  const quorum = b.quorum ?? 0
  const sizeBytes = b.size_bytes ?? b.header?.size_bytes

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      {/* ── Header ── */}
      <header style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', flexWrap: 'wrap' }}>
        <h1 style={{ margin: 0, fontSize: 22, fontFamily: 'var(--font-mono)', letterSpacing: '-0.02em' }}>Block #{height}</h1>
        <StatusBadge variant={isFinalized ? 'success' : 'warning'} pulse={!isFinalized}>
          {isFinalized ? 'Finalized' : (b.phase || 'Pending')}
        </StatusBadge>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
          <ShareButton />
          <button type="button" onClick={goPrev} style={navBtn} disabled={!canPrev} title="Previous block (←)">← Prev</button>
          <button type="button" onClick={goNext} style={navBtn} disabled={!canNext} title="Next block (→)">Next →</button>
        </div>
      </header>

      <p style={{ color: 'var(--text-tertiary)', fontSize: 11, margin: 0 }}>
        Keyboard: ← previous · → next
      </p>

      {/* ── Block fields ── */}
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <Row k="Hash"         v={<Hash value={b.hash} full showCopy />} />
        <Row k="Previous"     v={<Hash value={b.prev_hash || b.previous_hash || b.header?.prev_hash} kind="block-hash" full />} />
        <Row k="Timestamp"    v={<>{timestamp} <span style={{ color: 'var(--text-tertiary)', marginLeft: 8 }}><TimeAgo timestamp={timestamp} /></span></>} />
        <Row k="Producer"     v={
          <Link to={`/validator/${(b.producer || b.header?.proposer_id || '')}`} style={{ textDecoration: 'none' }}>
            <Hash value={b.producer || b.header?.proposer_id} kind="validator" full />
          </Link>
        } />
        <Row k="Tx count"     v={txs.length} />
        <Row k="Size"         v={sizeBytes ? `${Number(sizeBytes).toLocaleString()} B` : '—'} />
        {b.signature && <Row k="Signature" v={<Hash value={b.signature} full showCopy />} />}
      </section>

      {/* ── Consensus / quorum section ── */}
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-4)' }}>
        <h2 style={{ margin: 0, fontSize: 15 }}>Consensus</h2>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: 'var(--space-3)' }}>
          <Metric label="Votes" value={votes.length} />
          <Metric label="Quorum" value={quorum || '—'} />
          <Metric
            label="Status"
            value={isFinalized ? 'Finalized' : quorum && votes.length >= quorum ? 'Quorum reached' : 'Collecting votes'}
            accent={isFinalized ? 'success' : votes.length >= quorum && quorum > 0 ? 'success' : 'warning'}
          />
        </div>

        {/* Quorum progress bar */}
        {quorum > 0 && (
          <div>
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, color: 'var(--text-tertiary)', marginBottom: 4 }}>
              <span>Approvals: {votes.length} / {quorum}</span>
              <span>{Math.min(100, Math.round((votes.length / quorum) * 100))}%</span>
            </div>
            <div style={{ height: 8, background: 'var(--surface)', borderRadius: 4, overflow: 'hidden' }}>
              <div style={{
                width: `${Math.min(100, (votes.length / quorum) * 100)}%`,
                height: '100%',
                background: votes.length >= quorum ? 'var(--accent-success)' : 'var(--accent-warning)',
                borderRadius: 4,
                transition: 'width .3s ease-out',
              }} />
            </div>
          </div>
        )}

        {/* Signer list */}
        {votes.length > 0 && (
          <div>
            <div style={{ fontSize: 11, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.04em', marginBottom: 6 }}>Signers</div>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              {votes.map((v, i) => {
                const signer = typeof v === 'string' ? v : (v.validator_id || v.signer || v.voter || '')
                return (
                  <Link key={i} to={`/validator/${signer}`} style={{ textDecoration: 'none' }}>
                    <span style={{
                      display: 'inline-flex', alignItems: 'center', gap: 4,
                      padding: '3px 8px', borderRadius: 4,
                      border: '1px solid var(--accent-success)',
                      borderLeftWidth: 3,
                      fontSize: 10, fontFamily: 'var(--font-mono)',
                      color: 'var(--text-secondary)', background: 'var(--surface)',
                    }}>
                      <span style={{ width: 5, height: 5, borderRadius: '50%', background: 'var(--accent-success)' }} />
                      {signer ? `${signer.slice(0, 8)}…${signer.slice(-4)}` : `voter-${i}`}
                    </span>
                  </Link>
                )
              })}
            </div>
          </div>
        )}
      </section>

      {/* ── Transactions ── */}
      {txs.length > 0 && (
        <section>
          <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 15 }}>Transactions ({txs.length})</h2>
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
                {txs.map((t, i) => {
                  const canonical = t.canonical || t.package_canonical
                  const txStatus = t.status || (isFinalized ? 'finalized' : 'included')
                  return (
                    <tr key={canonical || t.hash || i}>
                      <td style={{ color: 'var(--text-tertiary)', fontFamily: 'var(--font-mono)' }}>{i}</td>
                      <td>
                        {canonical ? (
                          <Link to={`/tx/${encodeURIComponent(canonical)}`} style={{ color: 'var(--accent-primary-light)', fontFamily: 'var(--font-mono)', textDecoration: 'none' }}>
                            {canonical}
                          </Link>
                        ) : (
                          <Hash value={t.hash} kind="tx" full />
                        )}
                      </td>
                      <td><Hash value={t.publisher || t.revoked_by || t.validator_id} kind="publisher" start={6} end={4} /></td>
                      <td>
                        <StatusBadge variant={txStatus === 'finalized' ? 'success' : txStatus === 'revoked' ? 'error' : 'info'}>
                          {txStatus}
                        </StatusBadge>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </div>
        </section>
      )}

      {/* ── Raw JSON ── */}
      <details className="ce-card">
        <summary style={{ cursor: 'pointer', color: 'var(--text-secondary)', fontSize: 13, fontWeight: 600 }}>Raw JSON</summary>
        <pre style={{ marginTop: 'var(--space-3)', fontSize: 11, color: 'var(--text-secondary)', overflowX: 'auto', background: 'var(--bg-elevated)', padding: 'var(--space-3)', borderRadius: 'var(--radius-sm)' }}>
          {JSON.stringify(b, null, 2)}
        </pre>
      </details>
    </div>
  )
}

/* ── Helpers ── */
const navBtn = {
  padding: '6px 12px',
  background: 'var(--surface)',
  border: '1px solid var(--border)',
  borderRadius: 'var(--radius-sm)',
  color: 'var(--text-secondary)',
  fontSize: 12,
  cursor: 'pointer',
  transition: 'all var(--transition-fast)',
}

function Row({ k, v }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>{v ?? '—'}</span>
    </div>
  )
}

function Metric({ label, value, accent }) {
  const color = accent === 'success' ? 'var(--accent-success)' : accent === 'error' ? 'var(--accent-error)' : accent === 'warning' ? 'var(--accent-warning)' : 'var(--text-primary)'
  return (
    <div style={{ padding: '8px 10px', background: 'var(--surface)', borderRadius: 6 }}>
      <div style={{ fontSize: 10, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{label}</div>
      <div style={{ fontSize: 16, fontFamily: 'var(--font-mono)', color, marginTop: 2 }}>{value}</div>
    </div>
  )
}
