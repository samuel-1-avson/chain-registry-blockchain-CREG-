import React, { useState } from 'react'
import { useParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { PackageIntelligencePanel } from '../components/PackageIntelligencePanel.jsx'

const TABS = [
  { id: 'overview', label: 'Overview' },
  { id: 'analysis', label: 'Deep Analysis' },
]

export default function PackageDetail() {
  const { id } = useParams()
  const canonical = decodeURIComponent(id)
  const [tab, setTab] = useState('overview')
  const { data, error, loading, refetch } = useFetch((s) => nodeApi.package(canonical, s), { deps: [canonical] })

  if (loading && !data) return <SkeletonCard lines={10} />
  if (error) return <ErrorState error={error} onRetry={refetch} title="Package not found" />
  if (!data) return <EmptyState title="Package not found" description={canonical} />

  const p = data
  const risk = p.deterministic_risk

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header>
        <h1 style={{ margin: 0, fontSize: 18, fontFamily: 'var(--font-mono)', wordBreak: 'break-all' }}>{canonical}</h1>
        <div style={{ display: 'flex', gap: 'var(--space-2)', marginTop: 'var(--space-2)', flexWrap: 'wrap' }}>
          {p.revoked && <StatusBadge variant="error">Revoked</StatusBadge>}
          {p.status && <StatusBadge variant="neutral">{p.status}</StatusBadge>}
          {risk?.band && <StatusBadge variant="warning">Risk: {risk.band}</StatusBadge>}
        </div>
      </header>

      <nav style={{ display: 'flex', gap: 'var(--space-2)', borderBottom: '1px solid var(--border-subtle)', paddingBottom: 'var(--space-2)' }}>
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => setTab(t.id)}
            style={{
              padding: '6px 12px',
              fontSize: 13,
              fontWeight: tab === t.id ? 600 : 400,
              border: 'none',
              borderRadius: 6,
              cursor: 'pointer',
              background: tab === t.id ? 'var(--bg-elevated)' : 'transparent',
              color: tab === t.id ? 'var(--text-primary)' : 'var(--text-secondary)',
            }}
          >
            {t.label}
          </button>
        ))}
      </nav>

      {tab === 'overview' && (
        <>
          <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
            <Row k="Version" v={p.version || '—'} />
            <Row k="Publisher" v={<Hash value={p.publisher} kind="publisher" full />} />
            <Row k="IPFS cid" v={p.ipfs_cid ? <Hash value={p.ipfs_cid} full /> : '—'} />
            <Row k="Content hash" v={p.content_hash ? <Hash value={p.content_hash} full /> : '—'} />
            <Row k="Block hash" v={p.block_hash ? <Hash value={p.block_hash} kind="block-hash" full /> : '—'} />
            {risk && (
              <>
                <Row k="Deterministic score" v={String(risk.deterministic_score ?? risk.score ?? '—')} />
                <Row k="Advisory score" v={String(risk.advisory_score ?? '—')} />
                <Row k="Disposition" v={risk.disposition || '—'} />
              </>
            )}
            {p.evidence_digest && <Row k="Evidence digest" v={<Hash value={p.evidence_digest} full />} />}
            {p.analysis_bundles?.llm_prompt_profile_id && (
              <Row k="LLM prompt profile" v={p.analysis_bundles.llm_prompt_profile_id} />
            )}
          </section>
          <details className="ce-card">
            <summary style={{ cursor: 'pointer', color: 'var(--text-secondary)', fontSize: 13, fontWeight: 600 }}>Raw JSON</summary>
            <pre style={{ marginTop: 'var(--space-3)', fontSize: 11, color: 'var(--text-secondary)', overflowX: 'auto' }}>{JSON.stringify(p, null, 2)}</pre>
          </details>
        </>
      )}

      {tab === 'analysis' && <PackageIntelligencePanel canonical={canonical} />}
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
