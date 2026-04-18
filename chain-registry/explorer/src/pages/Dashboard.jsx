import React, { useCallback, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useChainStats, useRuntimeConfig } from '../hooks/useStats.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ErrorState } from '../components/ErrorState.jsx'
import { formatNumber } from '../utils/format.js'

function StatCard({ label, value, hint, variant = 'default' }) {
  return (
    <div className="ce-stat" style={variant === 'accent' ? { borderColor: 'var(--border-accent)' } : {}}>
      <span className="ce-stat-label">{label}</span>
      <span className="ce-stat-value">{value ?? '—'}</span>
      {hint && <span style={{ color: 'var(--text-tertiary)', fontSize: 11 }}>{hint}</span>}
    </div>
  )
}

export default function Dashboard({ recentEvents = [] }) {
  const stats = useChainStats(4000)
  const cfg = useRuntimeConfig()
  const blocks = usePolling((s) => nodeApi.blocks({ limit: 10 }, s), { intervalMs: 5000 })
  const bridge = usePolling((s) => nodeApi.bridgeStatus(s), { intervalMs: 10_000 })

  if (stats.error && !stats.data) {
    return <ErrorState error={stats.error} onRetry={stats.refetch} title="Unable to load chain stats" />
  }

  const s = stats.data || {}
  const latest = Array.isArray(blocks.data) ? blocks.data : (blocks.data?.blocks || [])

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <section aria-label="Network stats">
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))', gap: 'var(--space-4)' }}>
          <StatCard label="Chain height" value={s.current_height ?? '—'} hint={s.genesis_hash ? `genesis ${s.genesis_hash.slice(0, 10)}…` : ''} variant="accent" />
          <StatCard label="Finalized" value={s.finalized_height ?? '—'} hint={s.finalization_lag != null ? `lag ${s.finalization_lag} blocks` : ''} />
          <StatCard label="Validators" value={formatNumber(s.validator_count)} hint={s.active_validators != null ? `${s.active_validators} active` : ''} />
          <StatCard label="Total stake" value={formatNumber(s.total_stake_native)} hint="CREG" />
          <StatCard label="Packages" value={formatNumber(s.package_count)} hint={s.publisher_count ? `${s.publisher_count} publishers` : ''} />
          <StatCard label="Pending txs" value={formatNumber(s.pending_tx_count)} hint={s.mempool_bytes ? `${Math.round(s.mempool_bytes / 1024)} KB` : ''} />
        </div>
      </section>

      <section aria-label="Latest blocks and live events" style={{ display: 'grid', gridTemplateColumns: '2fr 1fr', gap: 'var(--space-6)' }}>
        <div className="ce-card">
          <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-4)' }}>
            <h2 style={{ margin: 0, fontSize: 14, color: 'var(--text-primary)' }}>Latest blocks</h2>
            <Link to="/blocks" style={{ fontSize: 12, color: 'var(--accent-primary-light)', textDecoration: 'none' }}>View all →</Link>
          </header>
          {blocks.loading && !latest.length ? (
            <SkeletonCard lines={5} />
          ) : latest.length === 0 ? (
            <p style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>No blocks yet.</p>
          ) : (
            <table className="ce-table">
              <thead>
                <tr>
                  <th>Height</th>
                  <th>Hash</th>
                  <th>Txs</th>
                  <th>Producer</th>
                  <th>Age</th>
                </tr>
              </thead>
              <tbody>
                {latest.map((b) => (
                  <tr key={b.height ?? b.hash}>
                    <td style={{ fontFamily: 'var(--font-mono)', fontWeight: 600 }}>
                      <Link to={`/block/${b.height}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>#{b.height}</Link>
                    </td>
                    <td><Hash value={b.hash} kind="block-hash" start={6} end={6} /></td>
                    <td style={{ color: 'var(--text-secondary)' }}>{b.tx_count ?? b.transactions?.length ?? 0}</td>
                    <td><Hash value={b.producer} kind="validator" start={6} end={4} /></td>
                    <td><TimeAgo timestamp={b.timestamp_ms ?? b.timestamp} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>

        <aside className="ce-card">
          <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-4)' }}>
            <h2 style={{ margin: 0, fontSize: 14 }}>Live events</h2>
            <Link to="/events" style={{ fontSize: 12, color: 'var(--accent-primary-light)', textDecoration: 'none' }}>Open feed →</Link>
          </header>
          {recentEvents.length === 0 ? (
            <p style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>Waiting for events…</p>
          ) : (
            <ul style={{ listStyle: 'none', padding: 0, margin: 0, display: 'grid', gap: 'var(--space-2)', maxHeight: 420, overflowY: 'auto' }}>
              {recentEvents.slice(0, 30).map((ev, i) => (
                <li key={i} style={{ fontSize: 12, padding: '8px 10px', background: 'var(--surface)', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 8 }}>
                    <StatusBadge variant="info">{ev.type || ev.kind || 'event'}</StatusBadge>
                    <TimeAgo timestamp={ev.ts || ev.timestamp_ms || Date.now()} />
                  </div>
                  {ev.height != null && (
                    <div style={{ marginTop: 4, color: 'var(--text-secondary)', fontFamily: 'var(--font-mono)', fontSize: 11 }}>
                      <Link to={`/block/${ev.height}`} style={{ color: 'var(--accent-primary-light)' }}>#{ev.height}</Link>
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
        </aside>
      </section>

      <section aria-label="Network context">
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))', gap: 'var(--space-4)' }}>
          <div className="ce-card">
            <h3 style={{ margin: '0 0 var(--space-3) 0', fontSize: 13, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>Bridge status</h3>
            {bridge.data ? (
              <div style={{ display: 'grid', gap: 6, fontSize: 12 }}>
                <Row k="L1 chain" v={bridge.data.l1_chain_id ?? bridge.data.chain_id ?? '—'} />
                <Row k="Last anchor" v={bridge.data.last_anchor_block ?? bridge.data.last_committed ?? '—'} />
                <Row k="Bridge signer" v={bridge.data.signer_address ?? '—'} mono />
              </div>
            ) : <p style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>Loading…</p>}
          </div>
          <div className="ce-card">
            <h3 style={{ margin: '0 0 var(--space-3) 0', fontSize: 13, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>Runtime</h3>
            {cfg.data ? (
              <div style={{ display: 'grid', gap: 6, fontSize: 12 }}>
                <Row k="Build" v={cfg.data.version ?? cfg.data.build ?? '—'} />
                <Row k="Chain ID" v={cfg.data.chain_id ?? '—'} />
                <Row k="Network" v={cfg.data.network ?? cfg.data.profile ?? '—'} />
              </div>
            ) : <p style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>Loading…</p>}
          </div>
        </div>
      </section>
    </div>
  )
}

function Row({ k, v, mono }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
      <span style={{ color: 'var(--text-tertiary)' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontFamily: mono ? 'var(--font-mono)' : 'inherit', fontSize: mono ? 11 : 12, overflow: 'hidden', textOverflow: 'ellipsis' }}>{String(v)}</span>
    </div>
  )
}
