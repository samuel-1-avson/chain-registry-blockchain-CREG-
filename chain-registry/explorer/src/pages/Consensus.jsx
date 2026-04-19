import React, { useMemo } from 'react'
import { Link } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { formatNumber } from '../utils/format.js'

const PHASES = ['collecting-votes', 'contested', 'quorum-reached']

const PHASE_META = {
  'collecting-votes': { label: 'Collecting votes', variant: 'warning', pulse: true },
  contested:          { label: 'Contested',        variant: 'error',   pulse: true },
  'quorum-reached':   { label: 'Quorum reached',   variant: 'success', pulse: false },
}

export default function Consensus() {
  const { data, error, loading, refetch } = usePolling(
    (s) => nodeApi.consensusState(s),
    { intervalMs: 2000 },
  )

  if (loading && !data) return <SkeletonCard lines={8} />
  if (error && !data) return <ErrorState error={error} onRetry={refetch} title="Could not load consensus state" />

  const rounds = data?.active_rounds || []
  const validators = data?.validators || []
  const quorum = data?.quorum ?? 0
  const total = data?.total_validators ?? validators.length
  const pending = data?.pending_count ?? 0
  const online = validators.filter((v) => v.status !== 'offline').length

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', flexWrap: 'wrap', gap: 12 }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>PBFT consensus</h1>
        <div style={{ display: 'flex', gap: 16, fontSize: 12, color: 'var(--text-tertiary)' }}>
          <Stat label="Validators" value={`${online}/${total} online`} />
          <Stat label="Quorum" value={`${quorum} of ${total}`} />
          <Stat label="Active rounds" value={rounds.length} />
          <Stat label="Pending pool" value={formatNumber(pending)} />
        </div>
      </header>

      {rounds.length === 0
        ? <EmptyState title="No active rounds" description="No PBFT rounds in flight — chain is at tip." />
        : rounds.map((r) => (
            <RoundCard key={r.block_hash} round={r} quorum={quorum} validators={validators} />
          ))}
    </div>
  )
}

function RoundCard({ round, quorum, validators }) {
  const { approvals, rejections, voters, approvers, rejecters, phase, age_ms } = round
  const meta = PHASE_META[phase] || { label: phase, variant: 'muted' }
  const progress = quorum ? Math.min(100, Math.round((approvals / quorum) * 100)) : 0
  const phaseIdx = PHASES.indexOf(phase)

  const voterSet = useMemo(() => new Set(voters || []), [voters])
  const approverSet = useMemo(() => new Set(approvers || []), [approvers])
  const rejecterSet = useMemo(() => new Set(rejecters || []), [rejecters])

  return (
    <article className="ce-card" style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, flexWrap: 'wrap' }}>
        <div>
          <div style={{ fontSize: 11, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: 4 }}>Block hash</div>
          <Hash value={round.block_hash} kind="block-hash" full />
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <span style={{ fontSize: 12, color: 'var(--text-tertiary)', fontVariantNumeric: 'tabular-nums' }}>
            {formatAge(age_ms)}
          </span>
          <StatusBadge variant={meta.variant} pulse={meta.pulse}>{meta.label}</StatusBadge>
        </div>
      </header>

      <PhasePipeline current={phaseIdx} />

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))', gap: 'var(--space-3)' }}>
        <Metric label="Approvals" value={`${approvals} / ${quorum || '?'}`} accent="success" />
        <Metric label="Rejections" value={rejections} accent={rejections > 0 ? 'error' : 'muted'} />
        <Metric label="Total votes" value={voters?.length ?? 0} />
        <Metric label="Quorum progress" value={`${progress}%`} />
      </div>

      <ProgressBar progress={progress} variant={approvals >= quorum ? 'success' : rejections > 0 ? 'error' : 'warning'} />

      <div>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
          <span style={{ fontSize: 11, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>Validator participation</span>
          <Legend />
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(140px, 1fr))', gap: 6 }}>
          {validators.map((v) => {
            const voted = voterSet.has(v.id)
            const approved = approverSet.has(v.id)
            const rejected = rejecterSet.has(v.id)
            const state = approved ? 'approved' : rejected ? 'rejected' : voted ? 'voted' : 'pending'
            return <VoterTile key={v.id} validator={v} state={state} />
          })}
        </div>
      </div>
    </article>
  )
}

function PhasePipeline({ current }) {
  const items = [
    { label: 'Voting', phase: 'collecting-votes' },
    { label: 'Review', phase: 'contested' },
    { label: 'Finalize', phase: 'quorum-reached' },
  ]
  return (
    <div style={{ display: 'grid', gridTemplateColumns: `repeat(${items.length}, 1fr)`, gap: 4 }}>
      {items.map((it, i) => {
        const reached = current >= i
        const active = current === i
        return (
          <div key={it.phase} style={{
            padding: '8px 10px',
            borderRadius: 6,
            background: reached ? 'var(--border-accent)' : 'var(--surface)',
            border: `1px solid ${active ? 'var(--accent-primary)' : 'var(--border)'}`,
            color: reached ? 'var(--text-primary)' : 'var(--text-tertiary)',
            fontSize: 12,
            textAlign: 'center',
            fontWeight: active ? 600 : 400,
            transition: 'background .2s, border-color .2s',
          }}>
            <span style={{ marginRight: 6 }}>{reached ? '●' : '○'}</span>
            {it.label}
          </div>
        )
      })}
    </div>
  )
}

function ProgressBar({ progress, variant }) {
  const color = variant === 'success'
    ? 'var(--accent-success)'
    : variant === 'error'
      ? 'var(--accent-error)'
      : 'var(--accent-warning)'
  return (
    <div style={{ height: 10, background: 'var(--surface)', borderRadius: 5, overflow: 'hidden' }}>
      <div style={{ width: `${progress}%`, height: '100%', background: color, transition: 'width .3s ease-out' }} />
    </div>
  )
}

function Metric({ label, value, accent }) {
  const color = accent === 'success'
    ? 'var(--accent-success)'
    : accent === 'error'
      ? 'var(--accent-error)'
      : 'var(--text-primary)'
  return (
    <div style={{ padding: '8px 10px', background: 'var(--surface)', borderRadius: 6 }}>
      <div style={{ fontSize: 10, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{label}</div>
      <div style={{ fontSize: 16, fontFamily: 'var(--font-mono)', color, marginTop: 2 }}>{value}</div>
    </div>
  )
}

function Stat({ label, value }) {
  return (
    <span>
      <span style={{ textTransform: 'uppercase', letterSpacing: '0.04em', marginRight: 6 }}>{label}</span>
      <span style={{ color: 'var(--text-primary)', fontFamily: 'var(--font-mono)' }}>{value}</span>
    </span>
  )
}

function Legend() {
  const items = [
    { color: 'var(--accent-success)', label: 'approved' },
    { color: 'var(--accent-error)',   label: 'rejected' },
    { color: 'var(--accent-warning)', label: 'voted' },
    { color: 'var(--border)',         label: 'pending' },
  ]
  return (
    <div style={{ display: 'flex', gap: 10, fontSize: 10, color: 'var(--text-tertiary)' }}>
      {items.map((it) => (
        <span key={it.label} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
          <span style={{ width: 8, height: 8, borderRadius: 2, background: it.color, display: 'inline-block' }} />
          {it.label}
        </span>
      ))}
    </div>
  )
}

function VoterTile({ validator, state }) {
  const color = state === 'approved'
    ? 'var(--accent-success)'
    : state === 'rejected'
      ? 'var(--accent-error)'
      : state === 'voted'
        ? 'var(--accent-warning)'
        : 'var(--border)'
  const textColor = state === 'pending' ? 'var(--text-tertiary)' : 'var(--text-primary)'
  return (
    <Link
      to={`/validator/${validator.id}`}
      title={`${validator.alias || validator.id} · ${state}${validator.status === 'self' ? ' · (self)' : ''}`}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 6,
        padding: '6px 8px',
        background: 'var(--surface)',
        border: `1px solid ${color}`,
        borderLeftWidth: 3,
        borderRadius: 4,
        fontSize: 11,
        color: textColor,
        textDecoration: 'none',
        fontFamily: 'var(--font-mono)',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}
    >
      <span style={{ width: 6, height: 6, borderRadius: '50%', background: color, flexShrink: 0 }} />
      <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>
        {validator.alias || shortId(validator.id)}
      </span>
      {validator.status === 'self' && <span style={{ color: 'var(--accent-primary-light)', fontSize: 9 }}>•you</span>}
    </Link>
  )
}

function shortId(id) {
  if (!id) return '—'
  if (id.length <= 12) return id
  return `${id.slice(0, 6)}…${id.slice(-4)}`
}

function formatAge(ms) {
  if (ms == null) return '—'
  const s = Math.max(0, Math.round(ms / 1000))
  if (s < 60) return `${s}s ago`
  const m = Math.floor(s / 60)
  return `${m}m ${s % 60}s ago`
}
