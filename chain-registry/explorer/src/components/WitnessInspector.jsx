import React from 'react'
import { useFetch } from '../hooks/useFetch.js'
import { nodeApi } from '../api/node.js'
import { formatViolationTitle } from '../api/normalize.js'
import { SkeletonCard } from './Skeleton.jsx'
import { ErrorState, EmptyState } from './ErrorState.jsx'
import { StatusBadge } from './StatusBadge.jsx'
import { CopyButton } from './CopyButton.jsx'
import { Hash } from './Hash.jsx'
import { UntrustedText } from './UntrustedText.jsx'

function verificationBadge(v) {
  switch (v?.kind) {
    case 'on_chain_certified':
      return { variant: 'success', text: `OnChainCertified#${v.height}` }
    case 'on_chain_uncertified':
      return { variant: 'warning', text: `OnChainUncertified#${v.height}` }
    case 'gossip_only':
    default:
      return { variant: 'muted', text: 'GossipOnly' }
  }
}

export function WitnessInspector({ canonical }) {
  const { data, error, loading, refetch } = useFetch(
    (s) => nodeApi.packageWitnesses(canonical, s),
    { deps: [canonical] },
  )

  if (loading && !data) return <SkeletonCard lines={6} />
  if (error) return <ErrorState error={error} onRetry={refetch} title="Trigger witnesses unavailable" />

  const witnesses = Array.isArray(data?.witnesses) ? data.witnesses : []
  if (witnesses.length === 0) {
    return (
      <EmptyState
        title="No Class B trigger witnesses"
        description="No proof-carrying sandbox trigger witnesses have been attached to this package."
      />
    )
  }

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      {witnesses.map((w) => {
        const badge = verificationBadge(w.verification)
        const cliCmd = `creg evidence verify-witness ${w.digest}`
        return (
          <article key={w.digest} className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
            <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                <StatusBadge variant={badge.variant}>{badge.text}</StatusBadge>
                <StatusBadge variant={w.hybrid ? 'success' : 'muted'}>
                  {w.hybrid ? 'Hybrid PQ' : 'Classical Ed25519'}
                </StatusBadge>
                <Hash value={w.digest} full />
              </div>
              <CopyButton value={cliCmd} label="Copy verify command" />
            </header>

            <div style={{ fontSize: 12, color: 'var(--text-secondary)', display: 'flex', gap: 16, flexWrap: 'wrap' }}>
              <span>
                Prover: <UntrustedText value={w.prover_id} maxGraphemes={64} />
              </span>
              <span>
                Key ID: <code>{w.prover_key_id}</code>
              </span>
              <span>
                Graph digest: <Hash value={w.graph_canonical_digest} start={8} end={6} />
              </span>
              <span>Clock offset: {w.clock_offset_secs ?? 0}s</span>
            </div>

            <div>
              <strong style={{ fontSize: 12, textTransform: 'uppercase', color: 'var(--text-tertiary)' }}>
                Observed Violations
              </strong>
              <ul style={{ margin: '6px 0 0 18px', padding: 0, display: 'grid', gap: 4, fontSize: 13 }}>
                {(w.violations || []).map((v, i) => (
                  <li key={`${v.code || v.unknown || i}`}>
                    <span>{formatViolationTitle(v)}</span>
                    {v.code && (
                      <code style={{ marginLeft: 8, fontSize: 11, color: 'var(--text-tertiary)' }}>{v.code}</code>
                    )}
                  </li>
                ))}
              </ul>
            </div>

            {w.entry_function && (
              <div style={{ fontSize: 13 }}>
                <strong style={{ color: 'var(--text-tertiary)', marginRight: 8 }}>Entry function:</strong>
                <UntrustedText value={w.entry_function} maxGraphemes={128} expandable />
              </div>
            )}

            {Array.isArray(w.args) && w.args.length > 0 && (
              <div style={{ display: 'grid', gap: 4, fontSize: 12 }}>
                <strong style={{ color: 'var(--text-tertiary)' }}>Trigger Arguments</strong>
                {w.args.map((arg, i) => (
                  <div key={i}>
                    <UntrustedText value={arg} maxGraphemes={128} expandable />
                  </div>
                ))}
              </div>
            )}

            {Array.isArray(w.env_overrides) && w.env_overrides.length > 0 && (
              <div style={{ display: 'grid', gap: 4, fontSize: 12 }}>
                <strong style={{ color: 'var(--text-tertiary)' }}>Environment Overrides</strong>
                {w.env_overrides.map(([k, v], i) => (
                  <div key={i} style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'baseline' }}>
                    <UntrustedText value={k} maxGraphemes={64} label="env-key" />
                    <span>=</span>
                    <UntrustedText value={v} maxGraphemes={128} expandable label="env-val" />
                  </div>
                ))}
              </div>
            )}

            <div style={{ fontSize: 11, color: 'var(--text-tertiary)', fontFamily: 'var(--font-mono)' }}>
              Offline verify: <code>{cliCmd}</code>
            </div>
          </article>
        )
      })}
    </div>
  )
}
