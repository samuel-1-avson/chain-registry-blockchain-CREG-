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
import { formatNumber, formatWei } from '../utils/format.js'

const PROPOSAL_STATUS = {
  active: { variant: 'warning', label: 'Active', pulse: true },
  passed: { variant: 'success', label: 'Passed', pulse: false },
  rejected: { variant: 'error', label: 'Rejected', pulse: false },
  queued: { variant: 'info', label: 'Queued', pulse: true },
  executed: { variant: 'success', label: 'Executed', pulse: false },
  cancelled: { variant: 'muted', label: 'Cancelled', pulse: false },
}

const PROPOSAL_TYPES = ['all', 'parameter', 'upgrade', 'treasury', 'security', 'other']

/* ── Vote progress bar ─────────────────────────────────────────────────────── */
function VoteBar({ yea, nay, abstain, quorum }) {
  const total = yea + nay + abstain
  if (total === 0) return <span style={{ color: 'var(--text-tertiary)', fontSize: 11 }}>No votes yet</span>
  const yeaPct = (yea / total) * 100
  const nayPct = (nay / total) * 100
  const absPct = (abstain / total) * 100
  const quorumPct = quorum && total > 0 ? Math.min(100, (total / quorum) * 100) : null
  return (
    <div>
      <div style={{ display: 'flex', gap: 12, fontSize: 10, color: 'var(--text-tertiary)', marginBottom: 4 }}>
        <span>Yea: <strong style={{ color: 'var(--accent-success)' }}>{formatNumber(yea)}</strong> ({yeaPct.toFixed(0)}%)</span>
        <span>Nay: <strong style={{ color: 'var(--accent-error)' }}>{formatNumber(nay)}</strong> ({nayPct.toFixed(0)}%)</span>
        <span>Abstain: <strong>{formatNumber(abstain)}</strong> ({absPct.toFixed(0)}%)</span>
        {quorum != null && <span style={{ marginLeft: 'auto' }}>Quorum: {formatNumber(total)} / {formatNumber(quorum)}</span>}
      </div>
      <div style={{ height: 8, display: 'flex', borderRadius: 4, overflow: 'hidden', background: 'var(--bg-elevated)' }}>
        <div style={{ width: `${yeaPct}%`, background: 'var(--accent-success)', transition: 'width .3s' }} />
        <div style={{ width: `${nayPct}%`, background: 'var(--accent-error)', transition: 'width .3s' }} />
        <div style={{ width: `${absPct}%`, background: 'var(--text-tertiary)', opacity: 0.4, transition: 'width .3s' }} />
      </div>
      {quorumPct != null && (
        <div style={{ fontSize: 10, color: 'var(--text-tertiary)', marginTop: 2, textAlign: 'right' }}>
          quorum: {quorumPct.toFixed(0)}%
        </div>
      )}
    </div>
  )
}

/* ── Proposal card ─────────────────────────────────────────────────────────── */
function ProposalCard({ proposal }) {
  const p = proposal
  const meta = PROPOSAL_STATUS[p.status] || { variant: 'muted', label: p.status || 'unknown', pulse: false }
  const created = p.created_at || p.submitted_at || p.timestamp

  return (
    <article className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
      <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, flexWrap: 'wrap' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontFamily: 'var(--font-mono)', fontSize: 14, fontWeight: 700, color: 'var(--accent-primary-light)' }}>
            #{p.id ?? p.proposal_id ?? '—'}
          </span>
          <StatusBadge variant={meta.variant} pulse={meta.pulse}>{meta.label}</StatusBadge>
          {p.type && <StatusBadge variant="muted">{p.type}</StatusBadge>}
        </div>
        <TimeAgo timestamp={created} />
      </header>

      <h3 style={{ margin: 0, fontSize: 15, color: 'var(--text-primary)' }}>{p.title || 'Untitled proposal'}</h3>
      {p.description && (
        <p style={{ color: 'var(--text-secondary)', fontSize: 12, margin: 0, lineHeight: 1.5, maxHeight: 80, overflow: 'hidden' }}>
          {p.description}
        </p>
      )}

      <VoteBar
        yea={p.votes_yea ?? p.yea ?? 0}
        nay={p.votes_nay ?? p.nay ?? 0}
        abstain={p.votes_abstain ?? p.abstain ?? 0}
        quorum={p.quorum}
      />

      <footer style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, flexWrap: 'wrap', fontSize: 11 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ color: 'var(--text-tertiary)' }}>Proposer:</span>
          <Link to={`/address/${p.proposer || p.author || ''}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>
            <Hash value={p.proposer || p.author} start={6} end={4} />
          </Link>
        </div>
        {p.execution_block && (
          <span style={{ color: 'var(--text-tertiary)' }}>
            Execute at: <Link to={`/block/${p.execution_block}`} style={{ color: 'var(--accent-primary-light)' }}>#{p.execution_block}</Link>
          </span>
        )}
        {p.deposit && (
          <span style={{ color: 'var(--text-tertiary)' }}>Deposit: {formatWei(p.deposit)} CREG</span>
        )}
      </footer>
    </article>
  )
}

/* ── Summary stats card ────────────────────────────────────────────────────── */
function GovStat({ label, value, hint }) {
  return (
    <div className="ce-stat">
      <span className="ce-stat-label">{label}</span>
      <span className="ce-stat-value">{value ?? '—'}</span>
      {hint && <span style={{ color: 'var(--text-tertiary)', fontSize: 10, marginTop: 2 }}>{hint}</span>}
    </div>
  )
}

/* ── Main page ─────────────────────────────────────────────────────────────── */
export default function Governance() {
  const [typeFilter, setTypeFilter] = useState('all')

  const { data, error, loading, refetch } = usePolling(
    (signal) => nodeApi.governanceProposals(signal),
    { intervalMs: 15_000 },
  )

  const proposals = useMemo(() => {
    const list = data?.proposals || (Array.isArray(data) ? data : [])
    if (typeFilter === 'all') return list
    return list.filter((p) => (p.type || 'other').toLowerCase() === typeFilter)
  }, [data, typeFilter])
  const proposalStatus = getEndpointStatus(data)

  // Aggregate stats
  const all = data?.proposals || (Array.isArray(data) ? data : [])
  const active = all.filter((p) => p.status === 'active').length
  const passed = all.filter((p) => p.status === 'passed' || p.status === 'executed').length
  const totalVotes = all.reduce((s, p) => s + (p.votes_yea ?? 0) + (p.votes_nay ?? 0) + (p.votes_abstain ?? 0), 0)

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      {/* Header */}
      <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, flexWrap: 'wrap' }}>
        <div>
          <h1 style={{ margin: 0, fontSize: 20 }}>Governance</h1>
          <p style={{ color: 'var(--text-tertiary)', fontSize: 12, marginTop: 4 }}>
            On-chain proposals and voting activity
          </p>
        </div>
        <ShareButton />
      </header>

      {/* Stats */}
      <section style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: 'var(--space-4)' }}>
        <GovStat label="Total proposals" value={formatNumber(all.length)} />
        <GovStat label="Active" value={formatNumber(active)} hint="awaiting votes" />
        <GovStat label="Passed" value={formatNumber(passed)} />
        <GovStat label="Total votes cast" value={formatNumber(totalVotes)} />
      </section>

      {/* Type filter */}
      <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
        {PROPOSAL_TYPES.map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => setTypeFilter(t)}
            style={{
              padding: '4px 12px',
              borderRadius: 'var(--radius-full)',
              border: `1px solid ${typeFilter === t ? 'var(--border-accent)' : 'var(--border)'}`,
              background: typeFilter === t ? 'rgba(99,102,241,0.12)' : 'transparent',
              color: typeFilter === t ? 'var(--accent-primary-light)' : 'var(--text-tertiary)',
              fontSize: 11, fontWeight: 600, cursor: 'pointer',
              textTransform: 'uppercase', letterSpacing: '0.03em',
              transition: 'all var(--transition-fast)',
            }}
          >
            {t}
          </button>
        ))}
      </div>

      {/* Content */}
      {proposalStatus && <EndpointStatusNotice status={proposalStatus} title="Governance proposals unavailable" />}
      {error && !all.length && <ErrorState error={error} onRetry={refetch} title="Could not load governance proposals" />}
      {loading && !all.length && <SkeletonCard lines={8} />}
      {!loading && proposals.length === 0 && (
        <EmptyState
          title={proposalStatus ? 'Governance proposals unavailable' : 'No proposals'}
          description={proposalStatus
            ? 'This node does not expose governance proposal data yet. The page stays available, but proposals cannot be listed until that endpoint is deployed.'
            : typeFilter !== 'all'
            ? `No "${typeFilter}" proposals found. Try another filter.`
            : 'No governance activity yet. Proposals are created by validators through on-chain transactions.'}
        />
      )}
      <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
        {proposals.map((p, i) => (
          <ProposalCard key={p.id ?? p.proposal_id ?? i} proposal={p} />
        ))}
      </div>
    </div>
  )
}
