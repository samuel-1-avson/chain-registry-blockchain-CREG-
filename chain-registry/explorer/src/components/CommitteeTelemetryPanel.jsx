import React, { useState } from 'react'
import { useFetch } from '../hooks/useFetch.js'
import { nodeApi } from '../api/node.js'
import { SkeletonCard } from './Skeleton.jsx'
import { StatusBadge } from './StatusBadge.jsx'
import { Hash } from './Hash.jsx'
import { UntrustedText } from './UntrustedText.jsx'

export function CommitteeTelemetryPanel() {
  const [epoch, setEpoch] = useState(0)
  const { data, error, loading, refetch } = useFetch(
    (s) => nodeApi.committeeDiversity(epoch, s),
    { deps: [epoch] },
  )

  const unavailable = Boolean(data?.__endpointStatus) || Boolean(error) || !data?.validator_set_hash
  const totalDecisions = data
    ? (data.approvals || 0) + (data.quarantine_reviews || 0) + (data.rejections || 0) + (data.abstentions || 0)
    : 0
  const abstentionPct = totalDecisions > 0
    ? ((100 * (data.abstentions || 0)) / totalDecisions).toFixed(1)
    : '0.0'

  return (
    <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
        <div>
          <h2 style={{ margin: 0, fontSize: 16 }}>Committee Diversity &amp; Telemetry</h2>
          <span style={{ fontSize: 11, color: 'var(--text-tertiary)' }}>
            Public chain-derived telemetry only. Operator zone health is available in <code>creg console</code>.
          </span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <label htmlFor="committee-epoch-input" style={{ fontSize: 12, color: 'var(--text-secondary)' }}>
            Epoch
          </label>
          <input
            id="committee-epoch-input"
            type="number"
            min={0}
            value={epoch}
            onChange={(e) => setEpoch(Math.max(0, Number.parseInt(e.target.value || '0', 10) || 0))}
            style={{
              width: 72,
              padding: '4px 8px',
              borderRadius: 'var(--radius-sm)',
              border: '1px solid var(--border)',
              background: 'var(--bg-elevated)',
              color: 'var(--text-primary)',
              fontFamily: 'var(--font-mono)',
            }}
          />
          <button
            type="button"
            onClick={refetch}
            style={{
              padding: '4px 10px',
              borderRadius: 'var(--radius-sm)',
              border: '1px solid var(--border)',
              background: 'var(--surface)',
              color: 'var(--text-secondary)',
              fontSize: 12,
              cursor: 'pointer',
            }}
          >
            Refresh
          </button>
        </div>
      </header>

      {loading && !data ? (
        <SkeletonCard lines={4} />
      ) : unavailable ? (
        <p style={{ margin: 0, fontSize: 13, color: 'var(--text-secondary)' }}>
          Committee diversity metrics for epoch {epoch} are not available from the indexer yet.
        </p>
      ) : (
        <div style={{ display: 'grid', gap: 'var(--space-3)' }}>
          <div style={{ fontSize: 12, color: 'var(--text-secondary)', display: 'flex', gap: 12, flexWrap: 'wrap', alignItems: 'center' }}>
            <span>Validator set hash: <Hash value={data.validator_set_hash} start={8} end={6} /></span>
            <span>Observations: <strong>{data.observations}</strong></span>
            <span>Abstention rate: <strong>{abstentionPct}%</strong></span>
          </div>
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            <StatusBadge variant="success">Approvals {data.approvals}</StatusBadge>
            <StatusBadge variant="warning">Quarantine review {data.quarantine_reviews}</StatusBadge>
            <StatusBadge variant="error">Rejections {data.rejections}</StatusBadge>
            <StatusBadge variant="muted">Abstentions {data.abstentions}</StatusBadge>
            <StatusBadge variant="info">
              Hybrid {data.hybrid_signers} / Classical {data.classical_signers}
            </StatusBadge>
          </div>
          {Array.isArray(data.model_families) && data.model_families.length > 0 && (
            <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', fontSize: 12 }}>
              <span style={{ color: 'var(--text-tertiary)' }}>Model families:</span>
              {data.model_families.map((mf, idx) => (
                <span
                  key={`${mf.model_family}-${idx}`}
                  style={{
                    padding: '2px 8px',
                    borderRadius: 4,
                    border: '1px solid var(--border)',
                  }}
                >
                  <UntrustedText value={mf.model_family} maxGraphemes={48} label="family" /> ({mf.votes})
                </span>
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  )
}
