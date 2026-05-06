import React, { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { getEndpointStatus, nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard, SkeletonRow } from '../components/Skeleton.jsx'
import { EndpointStatusNotice, ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ShareButton } from '../components/ShareButton.jsx'
import { Sparkline } from '../hooks/useSparkline.jsx'
import { formatNumber } from '../utils/format.js'

/* ── Commit timeline node ──────────────────────────────────────────────────── */
function CommitNode({ anchor, isLatest }) {
  const l1Url = import.meta.env.VITE_L1_EXPLORER_URL || 'https://etherscan.io'

  return (
    <div style={{
      display: 'grid', gridTemplateColumns: '32px 1fr', gap: 'var(--space-3)',
      paddingBottom: 'var(--space-4)',
      position: 'relative',
    }}>
      {/* Timeline line + dot */}
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', position: 'relative' }}>
        <div style={{
          width: 12, height: 12, borderRadius: '50%',
          background: isLatest ? 'var(--accent-success)' : 'var(--border)',
          border: `2px solid ${isLatest ? 'var(--accent-success)' : 'var(--border)'}`,
          boxShadow: isLatest ? '0 0 0 4px rgba(34,197,94,0.15)' : 'none',
          zIndex: 1,
        }} />
        <div style={{
          position: 'absolute', top: 14, bottom: 0, width: 2,
          background: 'var(--border)', left: '50%', transform: 'translateX(-50%)',
        }} />
      </div>

      {/* Anchor content */}
      <div className="ce-card" style={{ display: 'grid', gap: 'var(--space-2)' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, flexWrap: 'wrap' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            {isLatest && <StatusBadge variant="success" pulse>Latest</StatusBadge>}
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 13, fontWeight: 600 }}>
              L2 Block #{anchor.l2_height ?? anchor.block_height ?? '—'}
            </span>
          </div>
          <TimeAgo timestamp={anchor.committed_at || anchor.timestamp} />
        </div>
        <Row k="State root" v={anchor.state_root ? <Hash value={anchor.state_root} full showCopy /> : '—'} />
        <Row k="L1 tx hash" v={
          anchor.l1_tx_hash ? (
            <a href={`${l1Url}/tx/${anchor.l1_tx_hash}`} target="_blank" rel="noopener noreferrer" style={{ textDecoration: 'none' }}>
              <Hash value={anchor.l1_tx_hash} full showCopy />
            </a>
          ) : '—'
        } />
        <Row k="L1 block" v={anchor.l1_block ?? anchor.eth_block ?? '—'} />
        {anchor.gas_used && <Row k="Gas used" v={formatNumber(anchor.gas_used)} />}
        {anchor.l2_height != null && (
          <Link to={`/block/${anchor.l2_height}`} style={{ color: 'var(--accent-primary-light)', fontSize: 11, textDecoration: 'none' }}>
            View L2 block →
          </Link>
        )}
      </div>
    </div>
  )
}

/* ── Health indicator ──────────────────────────────────────────────────────── */
function HealthCard({ status }) {
  const s = status || {}
  const health = s.healthy ?? (s.last_anchor_block != null ? 'healthy' : 'unknown')
  const isHealthy = health === 'healthy' || health === true
  const syncStatus = s.bridge_sync_status || 'Unknown'

  const timeSinceAnchor = s.last_anchor_at
    ? Math.round((Date.now() - Date.parse(s.last_anchor_at)) / 60000)
    : null

  return (
    <div style={{
      display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))',
      gap: 'var(--space-3)',
    }}>
      <StatCard
        label="Health"
        value={isHealthy ? 'Healthy' : 'Degraded'}
        variant={isHealthy ? 'success' : 'warning'}
      />
      <StatCard label="Sync status" value={syncStatus} variant={syncStatus.toLowerCase() === 'synced' ? 'success' : 'warning'} />
      <StatCard label="Last anchor" value={s.last_anchor_block ?? '—'} />
      <StatCard label="L1 chain" value={s.l1_chain_id ?? s.chain_id ?? '—'} />
      <StatCard label="Commit cadence" value={s.commit_interval ?? '—'} />
      {timeSinceAnchor != null && (
        <StatCard label="Time since anchor" value={`${timeSinceAnchor}m ago`} variant={timeSinceAnchor > 30 ? 'error' : timeSinceAnchor > 10 ? 'warning' : 'success'} />
      )}
    </div>
  )
}

function StatCard({ label, value, variant }) {
  const borderColor = variant === 'success' ? 'var(--accent-success)' : variant === 'error' ? 'var(--accent-error)' : variant === 'warning' ? 'var(--accent-warning)' : 'var(--border)'
  return (
    <div style={{
      padding: 'var(--space-3) var(--space-4)',
      background: 'var(--surface)',
      border: '1px solid var(--border)',
      borderLeftWidth: 3,
      borderLeftColor: borderColor,
      borderRadius: 'var(--radius-sm)',
    }}>
      <div style={{ fontSize: 10, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{label}</div>
      <div style={{ fontSize: 16, fontFamily: 'var(--font-mono)', color: 'var(--text-primary)', marginTop: 2, fontWeight: 600 }}>{value ?? '—'}</div>
    </div>
  )
}

/* ── Main page ─────────────────────────────────────────────────────────────── */
export default function Bridge() {
  const { data, error, loading, refetch } = usePolling((s) => nodeApi.bridgeStatus(s), { intervalMs: 10_000 })
  const anchors = usePolling((s) => nodeApi.bridgeAnchors(s), { intervalMs: 30_000 })

  if (loading && !data) return <SkeletonCard lines={8} />
  if (error) return <ErrorState error={error} onRetry={refetch} title="Could not load bridge status" />

  const s = data || {}
  const anchorList = anchors.data?.anchors || (Array.isArray(anchors.data) ? anchors.data : [])
  const anchorStatus = getEndpointStatus(anchors.data)

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      {/* Header */}
      <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, flexWrap: 'wrap' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <h1 style={{ margin: 0, fontSize: 20 }}>L1 Bridge</h1>
          <StatusBadge variant={s.healthy ?? s.last_anchor_block ? 'success' : 'warning'}>
            {s.healthy || s.last_anchor_block ? 'Operational' : 'Unknown'}
          </StatusBadge>
        </div>
        <ShareButton />
      </header>

      {/* Health overview */}
      <HealthCard status={s} />

      {/* Bridge identity */}
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <h2 style={{ margin: 0, fontSize: 14 }}>Bridge configuration</h2>
        <Row k="Bridge contract" v={s.bridge_contract ? <Hash value={s.bridge_contract} full showCopy /> : '—'} />
        <Row k="Signer address" v={s.signer_address ? <Hash value={s.signer_address} full showCopy /> : '—'} />
        <Row k="Last anchor root" v={s.last_anchor_root ? <Hash value={s.last_anchor_root} full showCopy /> : '—'} />
        <Row k="Last anchor block" v={s.last_anchor_block ?? '—'} />
      </section>

      {/* Anchor history / commit timeline */}
      <section>
        <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-4)' }}>
          <h2 style={{ margin: 0, fontSize: 15 }}>Anchor history</h2>
          <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>
            {anchorList.length} commit{anchorList.length !== 1 ? 's' : ''}
          </span>
        </header>

        {anchorStatus && (
          <div style={{ marginBottom: 'var(--space-4)' }}>
            <EndpointStatusNotice status={anchorStatus} title="Anchor history unavailable" />
          </div>
        )}

        {anchors.loading && !anchorList.length ? (
          <SkeletonCard lines={6} />
        ) : anchors.error && !anchorList.length ? (
          <ErrorState error={anchors.error} onRetry={anchors.refetch} title="Could not load bridge anchor history" />
        ) : anchorList.length === 0 ? (
          <EmptyState
            title={anchorStatus ? 'Anchor history unavailable' : 'No anchor history'}
            description={anchorStatus
              ? 'The latest bridge status is still shown above, but historical L1 anchor entries cannot be listed until this node exposes the anchor-history endpoint.'
              : "The bridge hasn't committed any state to L1 yet."}
          />
        ) : (
          <div style={{ paddingLeft: 4 }}>
            {anchorList.map((a, i) => (
              <CommitNode key={a.l1_tx_hash || a.l2_height || i} anchor={a} isLatest={i === 0} />
            ))}
          </div>
        )}
      </section>
    </div>
  )
}

function Row({ k, v }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '180px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, wordBreak: 'break-all' }}>{v ?? '—'}</span>
    </div>
  )
}
