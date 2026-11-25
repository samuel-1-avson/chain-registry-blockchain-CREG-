import React from 'react'
import { Link } from 'react-router-dom'
import { useFetch } from '../hooks/useFetch.js'
import { nodeApi } from '../api/node.js'
import {
  findingCategoryLabel,
  packageStatusVariant,
  txDisplayStatus,
} from '../api/normalize.js'
import { SkeletonCard } from './Skeleton.jsx'
import { ErrorState, EmptyState } from './ErrorState.jsx'
import { StatusBadge } from './StatusBadge.jsx'
import { Hash } from './Hash.jsx'
import { UntrustedText } from './UntrustedText.jsx'

function CommitteeChips({ summary }) {
  if (!summary) return null
  const families = summary.model_families ? Object.entries(summary.model_families) : []
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, alignItems: 'center', fontSize: 11 }}>
      <StatusBadge variant="info">Committee {summary.effective_size ?? '?'}</StatusBadge>
      <StatusBadge variant="success">Approvals {summary.approvals ?? 0}</StatusBadge>
      <StatusBadge variant="warning">Quarantine review {summary.quarantine_reviews ?? 0}</StatusBadge>
      <StatusBadge variant="error">Rejections {summary.rejections ?? 0}</StatusBadge>
      <StatusBadge variant="muted">Abstentions {summary.abstentions ?? 0}</StatusBadge>
      <StatusBadge variant="muted">
        Hybrid {summary.hybrid_signers ?? 0} / Classical {summary.classical_signers ?? 0}
      </StatusBadge>
      {families.map(([family, count]) => (
        <span
          key={family}
          style={{
            padding: '2px 6px',
            borderRadius: 4,
            border: '1px solid var(--border)',
            color: 'var(--text-secondary)',
          }}
        >
          <UntrustedText value={family} maxGraphemes={48} label="family" /> ({count})
        </span>
      ))}
    </div>
  )
}

function StatusOverview({ view }) {
  const st = view?.status
  if (!st || !st.state) return null
  const variant = packageStatusVariant(st.state)
  const quorumLabel = view.quorum === 'v4_quorum' ? 'creg-vote-v4 quorum' : 'Legacy v3 quorum'

  return (
    <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
      <header style={{ display: 'flex', gap: 'var(--space-2)', alignItems: 'center', flexWrap: 'wrap' }}>
        <h2 style={{ margin: 0, fontSize: 16 }}>Current Status &amp; Authority</h2>
        <StatusBadge variant={variant}>{st.state}</StatusBadge>
        <StatusBadge variant="muted">{quorumLabel}</StatusBadge>
      </header>
      {st.state === 'quarantined' && (
        <div style={{ color: 'var(--accent-warning)', fontSize: 13 }}>
          Hold opened at <Link to={`/block/${st.height}`}>#{st.height}</Link> · Category:{' '}
          <strong>{findingCategoryLabel(st.reason_code)}</strong> · Fails closed to REVOKED at{' '}
          {new Date(st.expires_at_unix * 1000).toISOString().replace('T', ' ').slice(0, 16)} UTC
        </div>
      )}
      {st.state === 'revoked' && (
        <div style={{ display: 'grid', gap: 6, color: 'var(--accent-error)', fontSize: 13 }}>
          <div>
            Revoked at <Link to={`/block/${st.height}`}>#{st.height}</Link> · Authority:{' '}
            <code>{st.authority}</code>
          </div>
          {st.reason && (
            <div>
              Reason: <UntrustedText value={st.reason} maxGraphemes={256} expandable />
            </div>
          )}
          {st.certificate_digest && (
            <div>
              Committee QC digest: <Hash value={st.certificate_digest} full />
            </div>
          )}
          {st.witness_digest && (
            <div>
              Trigger witness digest: <Hash value={st.witness_digest} full />
            </div>
          )}
        </div>
      )}
      <CommitteeChips summary={st.committee || view.committee} />
    </section>
  )
}

export function PackageEvidenceTimeline({ canonical }) {
  const v4Query = useFetch((s) => nodeApi.packageV4(canonical, s), { deps: [canonical] })
  const historyQuery = useFetch((s) => nodeApi.packageEvidenceHistory(canonical, { limit: 50 }, s), {
    deps: [canonical],
  })

  if ((v4Query.loading || historyQuery.loading) && !v4Query.data && !historyQuery.data) {
    return <SkeletonCard lines={6} />
  }

  const v4 = v4Query.data?.__endpointStatus ? null : v4Query.data
  const hist = historyQuery.data?.__endpointStatus ? null : historyQuery.data
  const entries = hist?.entries || []

  if (!v4 && !entries.length && (v4Query.error || historyQuery.error)) {
    return (
      <ErrorState
        error={v4Query.error || historyQuery.error}
        onRetry={() => {
          v4Query.refetch()
          historyQuery.refetch()
        }}
        title="Evidence timeline unavailable"
      />
    )
  }

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      {v4 && <StatusOverview view={v4} />}
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <h3 style={{ margin: 0, fontSize: 15 }}>Status &amp; Revocation Timeline</h3>
          <span style={{ fontSize: 11, color: 'var(--text-tertiary)' }}>{entries.length} events</span>
        </header>
        {entries.length === 0 ? (
          <EmptyState
            title="No indexed evidence transitions yet"
            description="Evidence history populates once the standalone indexer processes status transitions for this package."
          />
        ) : (
          <div style={{ display: 'grid', gap: 'var(--space-3)' }}>
            {entries.map((entry, idx) => {
              const badge = txDisplayStatus(
                {
                  type: entry.source_tx_type,
                  package_status: entry.status,
                  resolution: entry.status === 'verified' ? 'released' : entry.status,
                },
                { finalized: true, included: true },
              )
              return (
                <article
                  key={`${entry.height}.${entry.tx_index}.${idx}`}
                  style={{
                    borderLeft: '2px solid var(--border)',
                    paddingLeft: 'var(--space-3)',
                    display: 'grid',
                    gap: 6,
                  }}
                >
                  <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
                    <StatusBadge variant={badge.variant}>{badge.label}</StatusBadge>
                    <StatusBadge variant="muted">{entry.source_tx_type}</StatusBadge>
                    <Link to={`/block/${entry.height}`} style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
                      #{entry.height}.{entry.tx_index}
                    </Link>
                    {entry.revocation_authority && (
                      <span style={{ fontSize: 11, color: 'var(--text-secondary)' }}>
                        authority: <code>{entry.revocation_authority}</code>
                      </span>
                    )}
                    {entry.reason_code && (
                      <span style={{ fontSize: 11, color: 'var(--text-secondary)' }}>
                        category: {findingCategoryLabel(entry.reason_code)}
                      </span>
                    )}
                  </div>
                  {entry.reason && (
                    <div style={{ fontSize: 12 }}>
                      <UntrustedText value={entry.reason} maxGraphemes={128} expandable />
                    </div>
                  )}
                  <CommitteeChips summary={entry.committee} />
                  {(entry.witness_digest || entry.certificate_digest) && (
                    <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', fontSize: 11, color: 'var(--text-tertiary)' }}>
                      {entry.witness_digest && (
                        <span>witness: <Hash value={entry.witness_digest} start={8} end={6} /></span>
                      )}
                      {entry.certificate_digest && (
                        <span>qc: <Hash value={entry.certificate_digest} start={8} end={6} /></span>
                      )}
                    </div>
                  )}
                </article>
              )
            })}
          </div>
        )}
      </section>
    </div>
  )
}
