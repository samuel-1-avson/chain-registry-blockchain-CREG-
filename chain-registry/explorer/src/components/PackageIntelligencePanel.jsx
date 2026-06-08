import React from 'react'
import { useFetch } from '../hooks/useFetch.js'
import { nodeApi } from '../api/node.js'
import { SkeletonCard } from './Skeleton.jsx'
import { ErrorState } from './ErrorState.jsx'
import { StatusBadge } from './StatusBadge.jsx'

function statusVariant(status) {
  switch (status) {
    case 'ready':
      return 'success'
    case 'degraded':
      return 'warning'
    case 'failed':
      return 'error'
    case 'pending':
    default:
      return 'neutral'
  }
}

function Section({ title, children }) {
  if (!children) return null
  return (
    <section style={{ display: 'grid', gap: 'var(--space-2)' }}>
      <h3 style={{ margin: 0, fontSize: 14, fontWeight: 600, color: 'var(--text-primary)' }}>{title}</h3>
      <div style={{ fontSize: 13, lineHeight: 1.55, color: 'var(--text-secondary)', whiteSpace: 'pre-wrap' }}>
        {children}
      </div>
    </section>
  )
}

function BulletList({ items }) {
  if (!items?.length) return <span style={{ color: 'var(--text-tertiary)' }}>None listed</span>
  return (
    <ul style={{ margin: 0, paddingLeft: 18, fontSize: 13, color: 'var(--text-secondary)' }}>
      {items.map((item, i) => (
        <li key={i} style={{ marginBottom: 6 }}>{item}</li>
      ))}
    </ul>
  )
}

export function PackageIntelligencePanel({ canonical }) {
  const { data, error, loading, refetch } = useFetch(
    (s) => nodeApi.packageIntelligence(canonical, s),
    { deps: [canonical] },
  )

  if (loading && !data) return <SkeletonCard lines={8} />
  if (error) return <ErrorState error={error} onRetry={refetch} title="Deep analysis unavailable" />

  const status = data?.status || 'pending'
  const report = data?.report

  return (
    <div className="ce-card" style={{ display: 'grid', gap: 'var(--space-5)' }}>
      <header style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', flexWrap: 'wrap' }}>
        <h2 style={{ margin: 0, fontSize: 16 }}>Deep Analysis</h2>
        <StatusBadge variant={statusVariant(status)}>{status}</StatusBadge>
        <span style={{ fontSize: 11, color: 'var(--text-tertiary)' }}>Lane C — advisory only, not consensus</span>
      </header>

      {status === 'pending' && (
        <p style={{ margin: 0, fontSize: 13, color: 'var(--text-secondary)' }}>
          {data?.message || 'Report is being generated or intelligence is disabled on this node.'}
        </p>
      )}

      {report && (
        <>
          {report.consensus_advisory && !report.consensus_advisory.degraded && (
            <div style={{ padding: 'var(--space-3)', borderRadius: 8, background: 'var(--bg-elevated)', fontSize: 12 }}>
              <strong>Lane B snapshot</strong> — score {report.consensus_advisory.maliciousness_score},{' '}
              tier {report.consensus_advisory.risk_tier}
              {report.consensus_advisory.model_used ? ` (${report.consensus_advisory.model_used})` : ''}
            </div>
          )}

          <Section title="Executive summary">{report.sections?.executive_summary}</Section>
          <Section title="What it does">{report.sections?.what_it_does}</Section>
          <Section title="Architecture">{report.sections?.architecture}</Section>
          <Section title="Supply chain">{report.sections?.supply_chain}</Section>
          <Section title="Security assessment">{report.sections?.security_assessment}</Section>

          <section>
            <h3 style={{ margin: '0 0 var(--space-2)', fontSize: 14 }}>Residual risks</h3>
            <BulletList items={report.sections?.residual_risks} />
          </section>

          <section>
            <h3 style={{ margin: '0 0 var(--space-2)', fontSize: 14 }}>Recommended actions</h3>
            <BulletList items={report.sections?.recommended_actions} />
          </section>

          {report.agent_trace?.length > 0 && (
            <details>
              <summary style={{ cursor: 'pointer', fontSize: 13, color: 'var(--text-secondary)' }}>
                Agent trace ({report.agent_trace.length} steps)
              </summary>
              <ol style={{ marginTop: 'var(--space-3)', paddingLeft: 20, fontSize: 12, color: 'var(--text-tertiary)' }}>
                {report.agent_trace.map((step) => (
                  <li key={step.step} style={{ marginBottom: 8 }}>
                    <code>{step.tool}</code> — {step.summary} ({step.duration_ms}ms)
                  </li>
                ))}
              </ol>
            </details>
          )}
        </>
      )}
    </div>
  )
}
