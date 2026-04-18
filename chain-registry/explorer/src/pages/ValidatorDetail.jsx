import React, { useMemo } from 'react'
import { Link, useParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard, SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ShareButton } from '../components/ShareButton.jsx'
import { formatWei, formatNumber, isEvmAddress } from '../utils/format.js'
import {
  AreaChart,
  Area,
  ResponsiveContainer,
  Tooltip,
} from 'recharts'

/* ── Performance stat card ─────────────────────────────────────────────────── */
function PerfCard({ label, value, hint, variant }) {
  const color = variant === 'success' ? 'var(--accent-success)' : variant === 'error' ? 'var(--accent-error)' : variant === 'warning' ? 'var(--accent-warning)' : 'var(--text-primary)'
  return (
    <div style={{
      padding: 'var(--space-3) var(--space-4)',
      background: 'var(--surface)',
      border: '1px solid var(--border)',
      borderLeftWidth: 3,
      borderLeftColor: color,
      borderRadius: 'var(--radius-sm)',
    }}>
      <div style={{ fontSize: 10, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{label}</div>
      <div style={{ fontSize: 18, fontFamily: 'var(--font-mono)', color, marginTop: 2, fontWeight: 700 }}>{value}</div>
      {hint && <div style={{ fontSize: 10, color: 'var(--text-tertiary)', marginTop: 2 }}>{hint}</div>}
    </div>
  )
}

/* ── Uptime gauge ──────────────────────────────────────────────────────────── */
function UptimeBar({ label, pct }) {
  const p = Math.max(0, Math.min(100, pct))
  const color = p >= 95 ? 'var(--accent-success)' : p >= 80 ? 'var(--accent-warning)' : 'var(--accent-error)'
  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, color: 'var(--text-tertiary)', marginBottom: 4 }}>
        <span>{label}</span>
        <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-primary)' }}>{p.toFixed(1)}%</span>
      </div>
      <div style={{ height: 6, background: 'var(--bg-elevated)', borderRadius: 3, overflow: 'hidden' }}>
        <div style={{ width: `${p}%`, height: '100%', background: color, borderRadius: 3, transition: 'width .3s' }} />
      </div>
    </div>
  )
}

export default function ValidatorDetail() {
  const { addr } = useParams()
  const address = (addr || '').toLowerCase()
  const valid = isEvmAddress(address)

  const profile = useFetch((s) => nodeApi.validatorProfile(address, s), {
    enabled: valid,
    deps: [address],
  })
  // Fetch address activity for slashing history and proposal stats
  const txs = useFetch((s) => nodeApi.addressTransactions(address, { limit: 200, scan: 1000 }, s), {
    enabled: valid,
    deps: [address],
  })

  if (!valid) {
    return <EmptyState title="Invalid address" description={`"${addr}" is not a valid EVM address.`} />
  }
  if (profile.error && !profile.data) {
    return <ErrorState error={profile.error} onRetry={profile.refetch} title="Validator not found" />
  }

  const p = profile.data || {}
  const reg = p.registration || null
  const proposals = p.recent_proposals || []
  const allTxs = txs.data?.transactions || []

  // Compute slashing events
  const slashEvents = useMemo(() => allTxs.filter((t) => t.kind === 'slash'), [allTxs])
  // Compute proposal sparkline (blocks per day)
  const proposalSparkData = useMemo(() => {
    if (proposals.length < 2) return []
    // Group proposals into daily buckets based on block height ranges
    const heightBucketSize = 100 // ~100 blocks per bucket
    const buckets = {}
    for (const b of proposals) {
      const bucket = Math.floor((b.height || b.header?.height || 0) / heightBucketSize)
      buckets[bucket] = (buckets[bucket] || 0) + 1
    }
    
    // Transform into recharts format
    return Object.entries(buckets).map(([k, count]) => ({
      name: `Bucket ${k}`,
      value: count,
    }))
  }, [proposals])

  // Estimate uptime from proposals vs total blocks
  const totalScanned = txs.data?.scanned_blocks || 0
  const proposalCount = allTxs.filter((t) => t.kind === 'propose').length
  const uptime7d = totalScanned > 0 ? Math.min(100, (proposalCount / Math.max(1, totalScanned / 10)) * 100) : null
  const voteCount = allTxs.filter((t) => t.kind === 'vote' || t.kind === 'consensus-vote').length

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      {/* ── Header ── */}
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 12, flexWrap: 'wrap' }}>
        <div>
          <h1 style={{ margin: 0, fontSize: 18, fontFamily: 'var(--font-mono)', wordBreak: 'break-all' }}>{address}</h1>
          <p style={{ color: 'var(--text-tertiary)', fontSize: 12, marginTop: 4 }}>
            Validator profile ·{' '}
            <Link to={`/address/${address}`} style={{ color: 'var(--accent-primary-light)' }}>full activity →</Link>
          </p>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <ShareButton />
          {p.in_active_set
            ? <StatusBadge variant="success">Active in set</StatusBadge>
            : <StatusBadge variant="muted">Not in active set</StatusBadge>}
        </div>
      </header>

      {/* ── Performance metrics ── */}
      <section>
        <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 15 }}>Performance</h2>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))', gap: 'var(--space-3)' }}>
          <PerfCard label="Blocks proposed" value={formatNumber(proposals.length)} hint={`in scan window`} variant="success" />
          <PerfCard
            label="Stake"
            value={`${formatWei(p.stake)} CREG`}
            variant={p.in_active_set ? 'success' : 'muted'}
          />
          <PerfCard label="Reputation" value={formatNumber(p.reputation ?? 0)} variant={p.reputation >= 80 ? 'success' : p.reputation >= 50 ? 'warning' : 'error'} />
          <PerfCard label="Status" value={p.status || '—'} variant={p.status === 'online' ? 'success' : p.status === 'self' ? 'info' : 'muted'} />
        </div>
      </section>

      {/* ── Uptime + sparkline ── */}
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-4)' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <h2 style={{ margin: 0, fontSize: 15 }}>Uptime & proposals</h2>
        </div>
        {proposalSparkData.length > 1 && (
          <div style={{ height: 120, width: '100%', margin: '16px 0' }}>
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={proposalSparkData}>
                <defs>
                  <linearGradient id="colorUptime" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor="var(--accent-success)" stopOpacity={0.3} />
                    <stop offset="95%" stopColor="var(--accent-success)" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <Tooltip
                  contentStyle={{ background: 'var(--surface)', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)' }}
                  labelStyle={{ color: 'var(--text-tertiary)' }}
                  itemStyle={{ color: 'var(--accent-success)', fontWeight: 600 }}
                />
                <Area type="monotone" dataKey="value" stroke="var(--accent-success)" strokeWidth={2} fillOpacity={1} fill="url(#colorUptime)" />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        )}
        {uptime7d != null && <UptimeBar label="Estimated uptime (scan window)" pct={uptime7d} />}
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(140px, 1fr))', gap: 'var(--space-3)' }}>
          <MiniStat label="Proposals scanned" value={proposalCount} />
          <MiniStat label="Votes observed" value={voteCount} />
          <MiniStat label="Blocks scanned" value={formatNumber(totalScanned)} />
          <MiniStat label="Slashing events" value={slashEvents.length} accent={slashEvents.length > 0 ? 'error' : undefined} />
        </div>
      </section>

      {/* ── Identity ── */}
      {profile.loading && !profile.data ? <SkeletonCard lines={6} /> : (
        <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
          <h2 style={{ margin: 0, fontSize: 15 }}>Identity</h2>
          <Row k="Alias"           v={reg?.alias || '—'} />
          {reg?.identity?.node_id && <Row k="Node ID"         v={<Hash value={reg.identity.node_id} full showCopy />} />}
          {reg?.identity?.ed25519_pubkey && <Row k="Ed25519 pubkey"  v={<Hash value={reg.identity.ed25519_pubkey} full showCopy />} />}
          {reg?.status && <Row k="Registration"    v={reg.status} />}
        </section>
      )}

      {/* ── Slashing history ── */}
      {slashEvents.length > 0 && (
        <section>
          <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 15, color: 'var(--accent-error)' }}>⚠ Slashing history</h2>
          <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
            <table className="ce-table">
              <thead>
                <tr>
                  <th>Height</th>
                  <th>Reason</th>
                  <th>Time</th>
                </tr>
              </thead>
              <tbody>
                {slashEvents.map((ev, i) => (
                  <tr key={i}>
                    <td style={{ fontFamily: 'var(--font-mono)' }}>
                      <Link to={`/block/${ev.block_height}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>#{ev.block_height}</Link>
                    </td>
                    <td style={{ color: 'var(--accent-error)', fontSize: 12 }}>{ev.canonical || 'slashed'}</td>
                    <td><TimeAgo timestamp={ev.timestamp} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      {/* ── Recent proposals ── */}
      <section className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
        <header style={{ padding: 'var(--space-3) var(--space-4)', borderBottom: '1px solid var(--border-subtle)', display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
          <h2 style={{ margin: 0, fontSize: 14 }}>Recent proposals</h2>
          <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{proposals.length}</span>
        </header>
        <table className="ce-table">
          <thead>
            <tr>
              <th style={{ width: 100 }}>Height</th>
              <th>Hash</th>
              <th style={{ width: 80 }}>Txs</th>
              <th style={{ width: 140 }}>Age</th>
            </tr>
          </thead>
          <tbody>
            {profile.loading && !proposals.length
              ? Array.from({ length: 4 }).map((_, i) => <SkeletonRow key={i} cells={4} />)
              : proposals.length === 0
                ? <tr><td colSpan={4} style={{ padding: 'var(--space-6)', textAlign: 'center', color: 'var(--text-tertiary)' }}>No recent proposals in the scan window.</td></tr>
                : proposals.map((b) => (
                  <tr key={b.height ?? b.hash ?? b.header?.height}>
                    <td style={{ fontFamily: 'var(--font-mono)', fontWeight: 600 }}>
                      <Link to={`/block/${b.height || b.header?.height}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>#{b.height || b.header?.height}</Link>
                    </td>
                    <td><Hash value={b.hash} kind="block-hash" start={8} end={6} /></td>
                    <td style={{ color: 'var(--text-secondary)' }}>{b.transactions?.length ?? b.tx_count ?? 0}</td>
                    <td><TimeAgo timestamp={b.header?.timestamp || b.timestamp_ms || b.timestamp} /></td>
                  </tr>
                ))}
          </tbody>
        </table>
      </section>
    </div>
  )
}

function MiniStat({ label, value, accent }) {
  const color = accent === 'error' ? 'var(--accent-error)' : 'var(--text-primary)'
  return (
    <div>
      <div style={{ fontSize: 10, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{label}</div>
      <div style={{ fontSize: 15, fontFamily: 'var(--font-mono)', color, marginTop: 2 }}>{value}</div>
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
