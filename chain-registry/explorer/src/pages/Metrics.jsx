import React, { useMemo, useState } from 'react'
import {
  ResponsiveContainer, AreaChart, Area, LineChart, Line,
  BarChart, Bar, XAxis, YAxis, CartesianGrid, Tooltip, Legend,
} from 'recharts'
import { nodeApi } from '../api/node.js'
import { useChainStats } from '../hooks/useStats.js'
import { EndpointStatusNotice, ErrorState, NoticeState } from '../components/ErrorState.jsx'
import { usePolling } from '../hooks/usePolling.js'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ShareButton } from '../components/ShareButton.jsx'
import { formatNumber } from '../utils/format.js'
import { getEndpointStatus } from '../api/node.js'

/* ── Chart theme (respects CSS variables) ──────────────────────────────────── */
const CHART_COLORS = {
  primary: '#e8a35c',
  success: '#22c55e',
  warning: '#f59e0b',
  error: '#ef4444',
  muted: '#64748b',
}

const tooltipStyle = {
  contentStyle: {
    background: 'var(--bg-elevated)',
    border: '1px solid var(--border)',
    borderRadius: 8,
    fontSize: 12,
    color: 'var(--text-primary)',
  },
}

/* ── Time-range selector ───────────────────────────────────────────────────── */
const TIME_RANGES = ['5m', '30m', '1h', '6h', '24h', '7d']

function TimeRangeSelector({ active, onChange }) {
  return (
    <div style={{ display: 'flex', gap: 4 }}>
      {TIME_RANGES.map((t) => (
        <button
          key={t}
          type="button"
          onClick={() => onChange(t)}
          style={{
            padding: '4px 10px',
            borderRadius: 'var(--radius-full)',
            border: `1px solid ${active === t ? 'var(--border-accent)' : 'var(--border)'}`,
            background: active === t ? 'rgba(232, 163, 92, 0.08)' : 'transparent',
            color: active === t ? 'var(--accent-primary-light)' : 'var(--text-tertiary)',
            fontSize: 10, fontWeight: 600, cursor: 'pointer',
            transition: 'all var(--transition-fast)',
          }}
        >
          {t}
        </button>
      ))}
    </div>
  )
}

/* ── Chart card wrapper ────────────────────────────────────────────────────── */
function ChartCard({ title, children, height = 240 }) {
  return (
    <div className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
      <h3 style={{ margin: 0, fontSize: 13, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>{title}</h3>
      <div style={{ width: '100%', height }}>
        {children}
      </div>
    </div>
  )
}

export default function Metrics() {
  const stats = useChainStats(5000)
  const [range, setRange] = useState('30m')
  const history = usePolling(
    (signal) => nodeApi.metricsHistory({ range }, signal),
    { intervalMs: 30_000, deps: [range] },
  )
  const chartData = useMemo(() => (history.data?.samples || []).flatMap((sample) => {
    const ts = Number(sample.timestamp_ms)
    if (!Number.isFinite(ts)) return []
    return [{
      ts,
      time: new Date(ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }),
      height: sample.height == null ? null : Number(sample.height),
      validators: sample.validator_count == null ? null : Number(sample.validator_count),
      activeValidators: sample.active_validators == null ? null : Number(sample.active_validators),
      packages: sample.package_count == null ? null : Number(sample.package_count),
      pending: sample.pending_tx_count == null ? null : Number(sample.pending_tx_count),
      publishers: sample.publisher_count == null ? null : Number(sample.publisher_count),
      totalStake: sample.total_stake == null ? null : Number(sample.total_stake),
    }]
  }), [history.data])

  // Compute deltas for TPS chart
  const tpsData = useMemo(() => {
    if (chartData.length < 2) return []
    return chartData.slice(1).map((c, i) => {
      const prev = chartData[i]
      const dtSec = Math.max(1, (c.ts - prev.ts) / 1000)
      const dHeight = c.height - prev.height
      return {
        time: c.time,
        blocksPerMin: Math.round((dHeight / dtSec) * 60 * 100) / 100,
        pendingDelta: c.pending - prev.pending,
      }
    })
  }, [chartData])

  const latestStats = stats.data || {}
  const historyStatus = getEndpointStatus(history.data)

  if (stats.error && !stats.data) return <ErrorState error={stats.error} onRetry={stats.refetch} title="Chain metrics unavailable" />

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      {/* Header */}
      <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, flexWrap: 'wrap' }}>
        <div>
          <h1 style={{ margin: 0, fontSize: 20 }}>Metrics</h1>
          <p style={{ color: 'var(--text-tertiary)', fontSize: 12, marginTop: 4 }}>
            {history.data?.status === 'ready'
              ? `Durable Prometheus history · ${chartData.length} stored samples · ${history.data.step_seconds}s resolution`
              : 'Durable history from server-side Prometheus scrapes'}
          </p>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <TimeRangeSelector active={range} onChange={setRange} />
          <ShareButton />
        </div>
      </header>

      {/* Live stat tiles */}
      <section style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: 'var(--space-3)' }}>
        <LiveTile label="Chain height" value={formatNumber(latestStats.current_height)} color={CHART_COLORS.primary} />
        <LiveTile label="Validators" value={`${latestStats.active_validators ?? '—'} / ${latestStats.validator_count ?? '—'}`} color={CHART_COLORS.success} />
        <LiveTile label="Packages" value={formatNumber(latestStats.package_count)} color={CHART_COLORS.warning} />
        <LiveTile label="Pending txs" value={formatNumber(latestStats.pending_tx_count)} color={CHART_COLORS.error} />
        <LiveTile label="Finalization lag" value={`${latestStats.finalization_lag ?? '—'} blocks`} color={latestStats.finalization_lag > 5 ? CHART_COLORS.error : CHART_COLORS.success} />
        <LiveTile label="Publishers" value={formatNumber(latestStats.publisher_count)} color={CHART_COLORS.muted} />
      </section>

      {historyStatus && <EndpointStatusNotice status={historyStatus} title="Historical metrics unavailable" />}
      {history.error && !history.data && (
        <NoticeState title="Historical metrics unavailable" description="The public history request failed. Live statistics above still come from this chain; no historical samples have been inferred." />
      )}
      {!historyStatus && !history.error && chartData.length < 3 ? (
        <div className="ce-card" style={{ textAlign: 'center', padding: 'var(--space-8)' }}>
          <p style={{ color: 'var(--text-tertiary)', fontSize: 13 }}>
            {history.loading
              ? 'Loading retained samples from the metrics collector…'
              : 'Collector is connected. Historical charts appear after it has stored enough real scrapes.'}
          </p>
          <SkeletonCard lines={6} />
        </div>
      ) : !historyStatus && !history.error && chartData.length >= 3 ? (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(400px, 1fr))', gap: 'var(--space-4)' }}>
          {/* Chain height */}
          <ChartCard title="Chain height">
            <ResponsiveContainer>
              <AreaChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                <XAxis dataKey="time" tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <YAxis tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} domain={['dataMin', 'dataMax']} />
                <Tooltip {...tooltipStyle} />
                <Area type="monotone" dataKey="height" stroke={CHART_COLORS.primary} fill={CHART_COLORS.primary} fillOpacity={0.1} strokeWidth={2} />
              </AreaChart>
            </ResponsiveContainer>
          </ChartCard>

          {/* Blocks/min (TPS proxy) */}
          <ChartCard title="Block production rate (blocks/min)">
            <ResponsiveContainer>
              <BarChart data={tpsData}>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                <XAxis dataKey="time" tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <YAxis tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <Tooltip {...tooltipStyle} />
                <Bar dataKey="blocksPerMin" fill={CHART_COLORS.success} radius={[2, 2, 0, 0]} />
              </BarChart>
            </ResponsiveContainer>
          </ChartCard>

          {/* Validator count */}
          <ChartCard title="Validators (active vs total)">
            <ResponsiveContainer>
              <LineChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                <XAxis dataKey="time" tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <YAxis tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <Tooltip {...tooltipStyle} />
                <Legend wrapperStyle={{ fontSize: 11 }} />
                <Line type="monotone" dataKey="validators" name="Total" stroke={CHART_COLORS.primary} strokeWidth={2} dot={false} />
                <Line type="monotone" dataKey="activeValidators" name="Active" stroke={CHART_COLORS.success} strokeWidth={2} dot={false} />
              </LineChart>
            </ResponsiveContainer>
          </ChartCard>

          {/* Pending pool */}
          <ChartCard title="Pending pool size">
            <ResponsiveContainer>
              <AreaChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                <XAxis dataKey="time" tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <YAxis tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <Tooltip {...tooltipStyle} />
                <Area type="monotone" dataKey="pending" stroke={CHART_COLORS.warning} fill={CHART_COLORS.warning} fillOpacity={0.1} strokeWidth={2} />
              </AreaChart>
            </ResponsiveContainer>
          </ChartCard>

          {/* Total stake */}
          <ChartCard title="Chain-authoritative total stake (CREG)">
            <ResponsiveContainer>
              <AreaChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                <XAxis dataKey="time" tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <YAxis tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <Tooltip {...tooltipStyle} />
                <Area type="monotone" dataKey="totalStake" stroke={CHART_COLORS.error} fill={CHART_COLORS.error} fillOpacity={0.1} strokeWidth={2} />
              </AreaChart>
            </ResponsiveContainer>
          </ChartCard>

          {/* Package count */}
          <ChartCard title="Total packages registered">
            <ResponsiveContainer>
              <AreaChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                <XAxis dataKey="time" tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} />
                <YAxis tick={{ fontSize: 10, fill: 'var(--text-tertiary)' }} domain={['dataMin', 'dataMax']} />
                <Tooltip {...tooltipStyle} />
                <Area type="monotone" dataKey="packages" stroke={CHART_COLORS.muted} fill={CHART_COLORS.muted} fillOpacity={0.1} strokeWidth={2} />
              </AreaChart>
            </ResponsiveContainer>
          </ChartCard>
        </div>
      ) : null}
    </div>
  )
}

function LiveTile({ label, value, color }) {
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
      <div style={{ fontSize: 18, fontFamily: 'var(--font-mono)', color: 'var(--text-primary)', marginTop: 2, fontWeight: 700 }}>{value ?? '—'}</div>
    </div>
  )
}
