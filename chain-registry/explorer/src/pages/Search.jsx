import React, { useEffect, useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'
import { nodeApi, ApiError } from '../api/node.js'
import { classifySearch, isHash32 } from '../utils/format.js'
import { Hash } from '../components/Hash.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { EmptyState } from '../components/ErrorState.jsx'

/**
 * /search?q=... — the classifier routes known shapes to their canonical URLs
 * directly; only ambiguous queries (32-byte hashes, free text) end up here.
 * For a 0x+64-hex hash we try block-by-hash, then fall back to tx.
 */
export default function Search() {
  const [params] = useSearchParams()
  const q = params.get('q') || ''
  const [matches, setMatches] = useState([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState(null)

  useEffect(() => {
    const cls = classifySearch(q)
    if (cls.kind !== 'hash') {
      setMatches([])
      return
    }
    let cancelled = false
    const controller = new AbortController()
    setLoading(true)
    setError(null)
    const tryLookup = async () => {
      const found = []
      try {
        const block = await nodeApi.blockByHash(cls.value, controller.signal)
        if (block) found.push({ kind: 'block', data: block, href: `/block/${block.height}` })
      } catch (e) {
        if (!(e instanceof ApiError) || e.status !== 404) {
          if (!cancelled) setError(e)
        }
      }
      try {
        const tx = await nodeApi.transaction(cls.value, controller.signal)
        if (tx) found.push({ kind: 'tx', data: tx, href: `/tx/${encodeURIComponent(cls.value)}` })
      } catch { /* not a tx, ignore */ }
      if (!cancelled) {
        setMatches(found)
        setLoading(false)
      }
    }
    tryLookup()
    return () => { cancelled = true; controller.abort() }
  }, [q])

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <h1 style={{ margin: 0, fontSize: 20 }}>Search results</h1>
      <p style={{ color: 'var(--text-tertiary)', fontSize: 13, margin: 0 }}>
        Query: <code style={{ fontFamily: 'var(--font-mono)' }}>{q}</code>
      </p>
      {loading && <p style={{ color: 'var(--text-secondary)' }}>Searching…</p>}
      {error && <p style={{ color: 'var(--accent-error)' }}>Lookup error: {error.message}</p>}
      {!loading && matches.length === 0 ? (
        <EmptyState
          title="No matches"
          description={isHash32(q)
            ? 'Not a known block hash or transaction hash.'
            : 'Try a block height, 0x address, tx canonical, or package name@version.'}
        />
      ) : (
        <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
          <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
            {matches.map((m, i) => (
              <li key={i} style={{ padding: 'var(--space-3) var(--space-4)', borderBottom: '1px solid var(--border)' }}>
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <StatusBadge variant="info">{m.kind}</StatusBadge>
                    <Link to={m.href} style={{ color: 'var(--accent-primary-light)', fontFamily: 'var(--font-mono)', fontSize: 12, textDecoration: 'none' }}>
                      {m.kind === 'block' ? `#${m.data.height}` : m.data.canonical || q}
                    </Link>
                  </div>
                  <Hash value={m.data.hash || m.data.canonical || q} start={10} end={6} showCopy={false} />
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
