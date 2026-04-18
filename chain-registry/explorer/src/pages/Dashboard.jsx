import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useChainStats, useRuntimeConfig } from '../hooks/useStats.js'
import { usePolling } from '../hooks/usePolling.js'
import { useSparkline, Sparkline } from '../hooks/useSparkline.jsx'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard, SkeletonRow } from '../components/Skeleton.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ErrorState } from '../components/ErrorState.jsx'
import { formatNumber, formatWei } from '../utils/format.js'

/* ── Stat card with optional sparkline ─────────────────────────────────────── */
function StatCard({ label, value, hint, variant = 'default', sparkData, animateValue }) {
  const prevVal = useRef(value)
  const [flash, setFlash] = useState(false)

  useEffect(() => {
    if (animateValue && prevVal.current !== value && value != null) {
      setFlash(true)
      const t = setTimeout(() => setFlash(false), 600)
      prevVal.current = value
      return () => clearTimeout(t)
    }
  }, [value, animateValue])

  return (
    <div
      className="ce-stat"
      style={{
        borderColor: variant === 'accent' ? 'var(--border-accent)' : undefined,
        transition: 'border-color .3s, box-shadow .3s',
        boxShadow: flash ? 'inset 0 0 20px rgba(99,102,241,0.10)' : undefined,
      }}
    >
      <span className="ce-stat-label">{label}</span>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <span className="ce-stat-value" style={{ transition: 'color .3s', color: flash ? 'var(--accent-primary-light)' : undefined }}>{value ?? '—'}</span>
        {sparkData && sparkData.length > 1 && (
          <Sparkline data={sparkData} width={80} height={28} color="var(--accent-primary-light)" />
        )}
      </div>
      {hint && <span style={{ color: 'var(--text-tertiary)', fontSize: 11 }}>{hint}</span>}
    </div>
  )
}

/* ── Finalization lag badge ─────────────────────────────────────────────────── */
function FinalizationLag({ lag }) {
  if (lag == null) return null
  const v = lag <= 2 ? 'success' : lag <= 5 ? 'warning' : 'error'
  return <StatusBadge variant={v}>{lag} block{lag !== 1 ? 's' : ''} behind</StatusBadge>
}

/* ── Event type filter chips ───────────────────────────────────────────────── */
const EVENT_TYPES = ['all', 'block', 'tx', 'vote', 'bridge', 'package']
function EventFilters({ active, onChange }) {
  return (
    <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
      {EVENT_TYPES.map((t) => (
        <button
          key={t}
          type="button"
          onClick={() => onChange(t)}
          style={{
            padding: '3px 10px',
            borderRadius: 'var(--radius-full)',
            border: `1px solid ${active === t ? 'var(--border-accent)' : 'var(--border)'}`,
            background: active === t ? 'rgba(99,102,241,0.12)' : 'transparent',
            color: active === t ? 'var(--accent-primary-light)' : 'var(--text-tertiary)',
            fontSize: 10,
            fontWeight: 600,
            cursor: 'pointer',
            textTransform: 'uppercase',
            letterSpacing: '0.04em',
            transition: 'all var(--transition-fast)',
          }}
        >
          {t}
        </button>
      ))}
    </div>
  )
}

/* ── Main Dashboard ────────────────────────────────────────────────────────── */
export default function Dashboard({ recentEvents = [] }) {
  const stats = useChainStats(4000)
  const cfg = useRuntimeConfig()
  const blocks = usePolling((s) => nodeApi.blocks({ limit: 10 }, s), { intervalMs: 5000 })
  const bridge = usePolling((s) => nodeApi.bridgeStatus(s), { intervalMs: 10_000 })
  const pkgs = usePolling((s) => nodeApi.packages({ limit: 5 }, s), { intervalMs: 15_000 })

  // Sparkline accumulators
  const validatorSpark = useSparkline({ maxSamples: 30 })
  const tpsSpark = useSparkline({ maxSamples: 30 })
  const prevHeight = useRef(null)
  const prevHeightTime = useRef(null)

  useEffect(() => {
    const s = stats.data
    if (!s) return
    if (s.validator_count != null) validatorSpark.push(s.validator_count)

    // TPS estimate: delta tx_count / delta time between polls
    const h = s.current_height
    if (prevHeight.current != null && h != null && h !== prevHeight.current) {
      const dtMs = Date.now() - (prevHeightTime.current || Date.now())
      const dtSec = Math.max(1, dtMs / 1000)
      const newBlocks = h - prevHeight.current
      const txPerBlock = (s.package_count || 0) / Math.max(1, h) // rough average
      const tps = (newBlocks * txPerBlock) / dtSec
      tpsSpark.push(Math.round(tps * 100) / 100)
    }
    prevHeight.current = h
    prevHeightTime.current = Date.now()
  }, [stats.data])

  // Event filter
  const [eventFilter, setEventFilter] = useState('all')
  const filteredEvents = useMemo(() => {
    if (eventFilter === 'all') return recentEvents
    return recentEvents.filter((ev) => {
      const kind = (ev.type || ev.kind || '').toLowerCase()
      if (eventFilter === 'block') return kind.includes('block')
      if (eventFilter === 'tx') return kind.includes('tx') || kind.includes('publish') || kind.includes('package')
      if (eventFilter === 'vote') return kind.includes('vote') || kind.includes('consensus')
      if (eventFilter === 'bridge') return kind.includes('bridge') || kind.includes('anchor')
      if (eventFilter === 'package') return kind.includes('package') || kind.includes('publish')
      return true
    })
  }, [recentEvents, eventFilter])

  if (stats.error && !stats.data) {
    return <ErrorState error={stats.error} onRetry={stats.refetch} title="Unable to load chain stats" />
  }

  const s = stats.data || {}
  const latest = Array.isArray(blocks.data) ? blocks.data : (blocks.data?.blocks || [])
  const packageList = pkgs.data?.packages || (Array.isArray(pkgs.data) ? pkgs.data : [])
  const finLag = s.finalization_lag ?? (s.current_height != null && s.finalized_height != null ? s.current_height - s.finalized_height : null)

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      {/* ── Stats row ── */}
      <section aria-label="Network stats">
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))', gap: 'var(--space-4)' }}>
          <StatCard
            label="Chain height"
            value={s.current_height ?? '—'}
            hint={s.genesis_hash ? `genesis ${s.genesis_hash.slice(0, 10)}…` : ''}
            variant="accent"
            animateValue
          />
          <StatCard
            label="Finalized"
            value={s.finalized_height ?? '—'}
            hint={finLag != null ? <FinalizationLag lag={finLag} /> : ''}
          />
          <StatCard
            label="Validators"
            value={formatNumber(s.validator_count)}
            hint={s.active_validators != null ? `${s.active_validators} active` : ''}
            sparkData={validatorSpark.data}
          />
          <StatCard
            label="Total stake"
            value={s.total_stake_native ? formatWei(s.total_stake_native) : formatNumber(s.total_stake)}
            hint="CREG"
            animateValue
          />
          <StatCard
            label="Packages"
            value={formatNumber(s.package_count)}
            hint={s.publisher_count ? `${s.publisher_count} publishers` : ''}
          />
          <StatCard
            label="Pending txs"
            value={formatNumber(s.pending_tx_count)}
            hint={s.mempool_bytes ? `${Math.round(s.mempool_bytes / 1024)} KB` : ''}
          />
        </div>
      </section>

      {/* ── Main content: blocks + events ── */}
      <section aria-label="Latest blocks and live events" style={{ display: 'grid', gridTemplateColumns: '2fr 1fr', gap: 'var(--space-6)' }}>
        {/* Block table */}
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
                    <td><Hash value={b.producer || b.header?.proposer_id} kind="validator" start={6} end={4} /></td>
                    <td><TimeAgo timestamp={b.timestamp_ms ?? b.timestamp ?? b.header?.timestamp} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>

        {/* Live events */}
        <aside className="ce-card" style={{ display: 'flex', flexDirection: 'column' }}>
          <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-3)' }}>
            <h2 style={{ margin: 0, fontSize: 14 }}>Live events</h2>
            <Link to="/events" style={{ fontSize: 12, color: 'var(--accent-primary-light)', textDecoration: 'none' }}>Open feed →</Link>
          </header>
          <EventFilters active={eventFilter} onChange={setEventFilter} />
          <div style={{ flex: 1, marginTop: 'var(--space-3)' }}>
            {filteredEvents.length === 0 ? (
              <p style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>
                {eventFilter === 'all' ? 'Waiting for events…' : `No "${eventFilter}" events yet.`}
              </p>
            ) : (
              <ul style={{ listStyle: 'none', padding: 0, margin: 0, display: 'grid', gap: 'var(--space-2)', maxHeight: 380, overflowY: 'auto' }}>
                {filteredEvents.slice(0, 30).map((ev, i) => (
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
          </div>
        </aside>
      </section>

      {/* ── Bottom row: recent packages + bridge + runtime ── */}
      <section aria-label="Network context">
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))', gap: 'var(--space-4)' }}>
          {/* Recent packages */}
          <div className="ce-card">
            <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-3)' }}>
              <h3 style={{ margin: 0, fontSize: 13, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>Recent packages</h3>
              <Link to="/packages" style={{ fontSize: 11, color: 'var(--accent-primary-light)', textDecoration: 'none' }}>View all →</Link>
            </header>
            {packageList.length === 0 ? (
              <p style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>No packages yet.</p>
            ) : (
              <ul style={{ listStyle: 'none', padding: 0, margin: 0, display: 'grid', gap: 6 }}>
                {packageList.slice(0, 5).map((p, i) => (
                  <li key={p.canonical || i} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 8, fontSize: 12 }}>
                    <Link to={`/package/${encodeURIComponent(p.canonical)}`} style={{ color: 'var(--accent-primary-light)', fontFamily: 'var(--font-mono)', fontSize: 11, textDecoration: 'none', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {p.canonical || p.name}
                    </Link>
                    <StatusBadge variant={p.status === 'verified' ? 'success' : p.status === 'revoked' ? 'error' : 'info'}>
                      {p.status || 'pending'}
                    </StatusBadge>
                  </li>
                ))}
              </ul>
            )}
          </div>

          {/* Bridge */}
          <div className="ce-card">
            <h3 style={{ margin: '0 0 var(--space-3) 0', fontSize: 13, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>Bridge status</h3>
            {bridge.data ? (
              <div style={{ display: 'grid', gap: 6, fontSize: 12 }}>
                <Row k="L1 chain" v={bridge.data.l1_chain_id ?? bridge.data.chain_id ?? '—'} />
                <Row k="Last anchor" v={bridge.data.last_anchor_block ?? bridge.data.last_committed ?? '—'} />
                <Row k="Bridge signer" v={bridge.data.signer_address ?? '—'} mono />
                <Row k="Sync status" v={
                  <StatusBadge variant={
                    (bridge.data.bridge_sync_status || '').toLowerCase() === 'synced' ? 'success'
                    : (bridge.data.bridge_sync_status || '').toLowerCase() === 'unknown' ? 'warning'
                    : 'muted'
                  }>
                    {bridge.data.bridge_sync_status || 'Unknown'}
                  </StatusBadge>
                } />
              </div>
            ) : <p style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>Loading…</p>}
          </div>

          {/* Runtime */}
          <div className="ce-card">
            <h3 style={{ margin: '0 0 var(--space-3) 0', fontSize: 13, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>Runtime</h3>
            {cfg.data ? (
              <div style={{ display: 'grid', gap: 6, fontSize: 12 }}>
                <Row k="Build" v={cfg.data.version ?? cfg.data.build ?? '—'} />
                <Row k="Chain ID" v={cfg.data.chain_id ?? '—'} />
                <Row k="Network" v={cfg.data.network ?? cfg.data.profile ?? '—'} />
                <Row k="Testnet" v={cfg.data.is_testnet ? 'Yes' : 'No'} />
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
      <span style={{ color: 'var(--text-primary)', fontFamily: mono ? 'var(--font-mono)' : 'inherit', fontSize: mono ? 11 : 12, overflow: 'hidden', textOverflow: 'ellipsis' }}>{typeof v === 'string' ? v : v ?? '—'}</span>
    </div>
  )
}
