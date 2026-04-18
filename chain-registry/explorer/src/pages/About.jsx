import React from 'react'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'

export default function About() {
  const cfg = useFetch((s) => nodeApi.runtimeConfig(s))
  const stats = useFetch((s) => nodeApi.chainStats(s))

  const c = cfg.data || {}
  const s = stats.data || {}

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header>
        <h1 style={{ margin: 0, fontSize: 22 }}>About this chain</h1>
        <p style={{ color: 'var(--text-tertiary)', fontSize: 13 }}>Protocol identity, contract addresses, and runtime configuration.</p>
      </header>
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <Row k="Chain ID"       v={c.chain_id ?? s.chain_id ?? '—'} />
        <Row k="Network"        v={c.network ?? c.profile ?? '—'} />
        <Row k="Version"        v={c.version ?? c.build ?? '—'} />
        <Row k="Genesis hash"   v={s.genesis_hash ? <Hash value={s.genesis_hash} full /> : '—'} />
        <Row k="Validator count" v={s.validator_count ?? '—'} />
      </section>
      <section className="ce-card">
        <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 14 }}>Explorer</h2>
        <p style={{ color: 'var(--text-secondary)', fontSize: 13 }}>
          This explorer ships as a deep-linkable SPA. Every page has a stable URL — copy and share any block, transaction, address, or package.
          The refactor follows the plan in <code style={{ fontSize: 11 }}>docs/EXPLORER_DEEP_DIVE.md</code>.
        </p>
      </section>

      <section className="ce-card">
        <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 14 }}>Developers & API</h2>
        <p style={{ color: 'var(--text-secondary)', fontSize: 13 }}>
          The node exposes a high-performance REST API. For a complete interactive reference of all available endpoints, queries, and data models, please visit the integrated <strong>Swagger UI</strong>.
        </p>
        <div style={{ marginTop: 'var(--space-4)' }}>
          <a
            href="/api-docs/"
            target="_blank"
            rel="noopener noreferrer"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 8,
              padding: '8px 16px',
              background: 'var(--accent-primary)',
              color: 'var(--text-primary)',
              textDecoration: 'none',
              borderRadius: 'var(--radius-md)',
              fontSize: 13,
              fontWeight: 600,
            }}
          >
            Open Swagger UI ↗
          </a>
        </div>
      </section>
    </div>
  )
}

function Row({ k, v }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '200px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, wordBreak: 'break-all' }}>{v}</span>
    </div>
  )
}
