import React, { useState } from 'react'
import { useParams, Link } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ShareButton } from '../components/ShareButton.jsx'

/* ── Status timeline steps ─────────────────────────────────────────────────── */
const TIMELINE_STEPS = ['pending', 'included', 'finalized']

function StatusTimeline({ status }) {
  const idx = TIMELINE_STEPS.indexOf(status)
  const isRevoked = status === 'revoked'

  const steps = isRevoked
    ? [...TIMELINE_STEPS, 'revoked']
    : TIMELINE_STEPS

  const activeIdx = isRevoked ? steps.length - 1 : Math.max(0, idx)

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 0, overflow: 'hidden' }}>
      {steps.map((step, i) => {
        const reached = i <= activeIdx
        const active = i === activeIdx
        const isRevokedStep = step === 'revoked'
        const color = isRevokedStep ? 'var(--accent-error)' : reached ? 'var(--accent-success)' : 'var(--border)'
        return (
          <React.Fragment key={step}>
            {i > 0 && (
              <div style={{
                flex: 1, height: 2, minWidth: 24,
                background: reached ? (isRevokedStep ? 'var(--accent-error)' : 'var(--accent-success)') : 'var(--border)',
                transition: 'background .3s',
              }} />
            )}
            <div style={{
              display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4, flexShrink: 0,
            }}>
              <div style={{
                width: active ? 20 : 14, height: active ? 20 : 14,
                borderRadius: '50%',
                border: `2px solid ${color}`,
                background: reached ? color : 'var(--bg)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                transition: 'all .3s',
                boxShadow: active ? `0 0 0 4px ${color}20` : 'none',
              }}>
                {reached && <span style={{ color: '#fff', fontSize: 9, fontWeight: 700 }}>✓</span>}
              </div>
              <span style={{
                fontSize: 10, fontWeight: active ? 700 : 500,
                color: reached ? 'var(--text-primary)' : 'var(--text-tertiary)',
                textTransform: 'capitalize',
              }}>{step}</span>
            </div>
          </React.Fragment>
        )
      })}
    </div>
  )
}

/* ── Validation score bar ──────────────────────────────────────────────────── */
function ScoreBar({ label, value, maxValue = 100 }) {
  const num = typeof value === 'number' ? value : parseFloat(value)
  if (!Number.isFinite(num)) return <Row k={label} v={formatValidationValue(value)} />
  const pct = Math.min(100, Math.max(0, (num / maxValue) * 100))
  const variant = pct >= 80 ? 'var(--accent-success)' : pct >= 50 ? 'var(--accent-warning)' : 'var(--accent-error)'
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{label}</span>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <div style={{ flex: 1, height: 6, background: 'var(--surface)', borderRadius: 3, overflow: 'hidden', maxWidth: 200 }}>
          <div style={{ width: `${pct}%`, height: '100%', background: variant, borderRadius: 3, transition: 'width .3s' }} />
        </div>
        <span style={{ fontFamily: 'var(--font-mono)', fontSize: 12, color: 'var(--text-primary)', minWidth: 32 }}>{num}</span>
      </div>
    </div>
  )
}

/** /tx/:canonical — canonical may be `name@version` or a 0x hash. */
export default function TxDetail() {
  const { id } = useParams()
  const canonical = decodeURIComponent(id)
  const [showProof, setShowProof] = useState(false)

  const { data, error, loading, refetch } = useFetch(
    (signal) => nodeApi.transaction(canonical, signal),
    { deps: [canonical] },
  )

  const proof = useFetch(
    (signal) => nodeApi.packageProof(canonical, signal),
    { deps: [canonical], enabled: showProof },
  )

  if (loading && !data) return <SkeletonCard lines={10} />
  if (error) return <ErrorState error={error} onRetry={refetch} title="Transaction not found" />
  if (!data) return <EmptyState title="Transaction not found" description={canonical} />

  // Normalize — the API may return { transaction, block_height, block_hash } or a flat object
  const t = data.transaction || data
  const blockHeight = data.block_height ?? t.block_height
  const blockHash = data.block_hash ?? t.block_hash
  const status = t.status || (blockHeight != null ? (t.finalized ? 'finalized' : 'included') : 'pending')

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      {/* ── Header ── */}
      <header style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
        <h1 style={{ margin: 0, fontSize: 18, fontFamily: 'var(--font-mono)', wordBreak: 'break-all' }}>{canonical}</h1>
        <StatusBadge variant={status === 'finalized' ? 'success' : status === 'revoked' ? 'error' : 'info'}>{status}</StatusBadge>
        <div style={{ marginLeft: 'auto' }}>
          <ShareButton />
        </div>
      </header>

      {/* ── Status timeline ── */}
      <section className="ce-card">
        <div style={{ fontSize: 11, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.04em', marginBottom: 'var(--space-3)' }}>Transaction lifecycle</div>
        <StatusTimeline status={status} />
      </section>

      {/* ── Core fields ── */}
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <Row k="Canonical"  v={t.canonical || canonical} mono />
        <Row k="Version"    v={t.version || (t.id?.version)} />
        <Row k="Publisher"  v={
          t.publisher || t.publisher_pubkey ? (
            <Link to={`/publisher/${encodeURIComponent(t.publisher || t.publisher_pubkey)}`} style={{ textDecoration: 'none' }}>
              <Hash value={t.publisher || t.publisher_pubkey} kind="publisher" full />
            </Link>
          ) : '—'
        } />
        <Row k="Block" v={
          blockHeight != null ? (
            <span style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Link to={`/block/${blockHeight}`} style={{ color: 'var(--accent-primary-light)' }}>#{blockHeight}</Link>
              {blockHash && <Hash value={blockHash} start={8} end={6} showCopy />}
              <StatusBadge variant={status === 'finalized' ? 'success' : 'info'}>{status}</StatusBadge>
            </span>
          ) : 'pending'
        } />
        <Row k="Included at" v={t.included_at ? <TimeAgo timestamp={t.included_at} /> : t.published_at ? <TimeAgo timestamp={t.published_at} /> : '—'} />
        <Row k="IPFS cid"   v={t.ipfs_cid ? <Hash value={t.ipfs_cid} full showCopy /> : '—'} />
        <Row k="Payload hash" v={t.payload_hash || t.content_hash ? <Hash value={t.payload_hash || t.content_hash} full showCopy /> : '—'} />
        {t.ecosystem && <Row k="Ecosystem" v={t.ecosystem || t.id?.ecosystem} />}
      </section>

      {/* ── Validation report ── */}
      {t.validation && (
        <section>
          <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 15 }}>Validation report</h2>
          <div className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
            {Object.entries(t.validation).map(([k, v]) => {
              if (typeof v === 'number' && v >= 0 && v <= 100) {
                return <ScoreBar key={k} label={k.replace(/_/g, ' ')} value={v} />
              }
              return <Row key={k} k={k.replace(/_/g, ' ')} v={formatValidationValue(v)} />
            })}
          </div>
        </section>
      )}

      {/* ── Proof section ── */}
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <h2 style={{ margin: 0, fontSize: 15 }}>Merkle proof</h2>
          <button
            type="button"
            onClick={() => setShowProof(true)}
            disabled={showProof && proof.loading}
            style={{
              padding: '6px 14px',
              background: 'var(--surface)',
              border: '1px solid var(--border)',
              borderRadius: 'var(--radius-sm)',
              color: 'var(--accent-primary-light)',
              fontSize: 12, fontWeight: 600, cursor: 'pointer',
              transition: 'all var(--transition-fast)',
            }}
          >
            {proof.loading ? 'Fetching…' : showProof && proof.data ? 'Loaded ✓' : '⇩ Load proof'}
          </button>
        </div>

        {showProof && proof.error && (
          <p style={{ color: 'var(--accent-error)', fontSize: 12 }}>
            Proof not available: {proof.error.message || 'endpoint error'}
          </p>
        )}

        {showProof && proof.data && (
          <>
            {proof.data.root && <Row k="Merkle root" v={<Hash value={proof.data.root} full showCopy />} />}
            {proof.data.path && Array.isArray(proof.data.path) && (
              <div>
                <div style={{ fontSize: 11, color: 'var(--text-tertiary)', textTransform: 'uppercase', marginBottom: 4 }}>Proof path ({proof.data.path.length} nodes)</div>
                <div style={{ display: 'grid', gap: 4 }}>
                  {proof.data.path.map((node, i) => (
                    <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11 }}>
                      <span style={{ color: 'var(--text-tertiary)', fontFamily: 'var(--font-mono)', minWidth: 24 }}>{i}</span>
                      <Hash value={typeof node === 'string' ? node : node.hash} start={10} end={8} showCopy />
                      {typeof node === 'object' && node.direction && (
                        <StatusBadge variant="muted">{node.direction}</StatusBadge>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}
            <button
              type="button"
              onClick={() => {
                const blob = new Blob([JSON.stringify(proof.data, null, 2)], { type: 'application/json' })
                const url = URL.createObjectURL(blob)
                const a = document.createElement('a')
                a.href = url
                a.download = `proof-${canonical.replace(/[^a-zA-Z0-9@._-]/g, '_')}.json`
                a.click()
                URL.revokeObjectURL(url)
              }}
              style={{
                padding: '6px 14px',
                background: 'var(--surface)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--radius-sm)',
                color: 'var(--text-secondary)',
                fontSize: 12, cursor: 'pointer',
              }}
            >
              ⇩ Download proof JSON
            </button>
          </>
        )}
      </section>

      {/* ── Raw JSON ── */}
      <details className="ce-card">
        <summary style={{ cursor: 'pointer', color: 'var(--text-secondary)', fontSize: 13, fontWeight: 600 }}>Raw JSON</summary>
        <pre style={{ marginTop: 'var(--space-3)', fontSize: 11, color: 'var(--text-secondary)', overflowX: 'auto', background: 'var(--bg-elevated)', padding: 'var(--space-3)', borderRadius: 'var(--radius-sm)' }}>
          {JSON.stringify(data, null, 2)}
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
