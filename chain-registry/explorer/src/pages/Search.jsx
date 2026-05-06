import React, { useEffect, useMemo, useState, useCallback } from 'react'
import { Link, useSearchParams, useNavigate } from 'react-router-dom'
import { getEndpointStatus, nodeApi } from '../api/node.js'
import { classifySearch, isHash32, isEvmAddress, isBlockHeight, isPackageCanonical } from '../utils/format.js'
import { Hash } from '../components/Hash.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { EmptyState, EndpointStatusNotice, NoticeState } from '../components/ErrorState.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'

const KIND_ICON = {
  block: '⬡',
  tx: '⟷',
  address: '◉',
  validator: '⬢',
  package: '📦',
  publisher: '🔑',
}

const KIND_COLOR = {
  block: 'info',
  tx: 'success',
  address: 'warning',
  validator: 'info',
  package: 'success',
  publisher: 'muted',
}

/**
 * /search?q=... — comprehensive search results page.
 *
 * Tries server-side /v1/search first, then falls back to client-side
 * multi-source lookup (block-by-hash, transaction, address, package).
 */
export default function Search() {
  const [params] = useSearchParams()
  const nav = useNavigate()
  const q = params.get('q') || ''
  const [matches, setMatches] = useState([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState(null)
  const [searchStatus, setSearchStatus] = useState(null)
  const [selected, setSelected] = useState(0)

  useEffect(() => {
    if (!q.trim()) {
      setMatches([])
      setError(null)
      setSearchStatus(null)
      return
    }
    let cancelled = false
    const controller = new AbortController()
    setLoading(true)
    setError(null)
    setSearchStatus(null)
    setSelected(0)

    const doSearch = async () => {
      const found = []
      let nextStatus = null
      let serverError = null

      // 1. Try server-side search
      try {
        const serverResult = await nodeApi.search(q, controller.signal)
        nextStatus = getEndpointStatus(serverResult)
        if (serverResult?.matches?.length > 0) {
          found.push(...serverResult.matches.map((m) => ({
            kind: m.kind,
            title: m.title || m.href,
            subtitle: m.subtitle || '',
            href: m.href,
            data: m,
          })))
        }
      } catch (err) {
        if (!controller.signal.aborted) serverError = err
      }

      // 2. Client-side fallback — try multiple lookups
      const cls = classifySearch(q)

      if (cls.kind === 'block-height') {
        try {
          const block = await nodeApi.blockByHeight(cls.value, controller.signal)
          if (block && !found.some((f) => f.kind === 'block')) {
            found.push({ kind: 'block', title: `Block #${block.height ?? cls.value}`, subtitle: block.hash ? `hash: ${block.hash.slice(0, 16)}…` : '', href: `/block/${cls.value}`, data: block })
          }
        } catch { /* not found */ }
      }

      if (cls.kind === 'address') {
        found.push({ kind: 'address', title: cls.value, subtitle: 'EVM address', href: `/address/${cls.value}`, data: null })
        // Also check if this is a validator
        try {
          const vp = await nodeApi.validatorProfile(cls.value, controller.signal)
          if (vp && !found.some((f) => f.kind === 'validator')) {
            found.push({ kind: 'validator', title: vp.registration?.alias || cls.value, subtitle: `stake: ${vp.stake || '0'}`, href: `/validator/${cls.value}`, data: vp })
          }
        } catch { /* not a validator */ }
      }

      if (cls.kind === 'hash') {
        // Try block by hash
        try {
          const block = await nodeApi.blockByHash(cls.value, controller.signal)
          if (block) found.push({ kind: 'block', title: `Block #${block.height}`, subtitle: `hash: ${cls.value.slice(0, 16)}…`, href: `/block/${block.height}`, data: block })
        } catch { /* not a block hash */ }
        // Try transaction
        try {
          const tx = await nodeApi.transaction(cls.value, controller.signal)
          if (tx) found.push({ kind: 'tx', title: tx.transaction?.canonical || tx.canonical || cls.value, subtitle: `block: ${tx.block_height ?? '?'}`, href: `/tx/${encodeURIComponent(cls.value)}`, data: tx })
        } catch { /* not a tx */ }
      }

      if (cls.kind === 'package') {
        try {
          const pkg = await nodeApi.package(cls.value, controller.signal)
          if (pkg) found.push({ kind: 'package', title: pkg.canonical || cls.value, subtitle: `status: ${pkg.status || 'unknown'}`, href: `/package/${encodeURIComponent(cls.value)}`, data: pkg })
        } catch { /* not found */ }
      }

      // Free text — also try as package
      if (cls.kind === 'text') {
        try {
          const pkg = await nodeApi.package(cls.value, controller.signal)
          if (pkg) found.push({ kind: 'package', title: pkg.canonical || cls.value, subtitle: `status: ${pkg.status || 'unknown'}`, href: `/package/${encodeURIComponent(cls.value)}`, data: pkg })
        } catch { /* ignore */ }
      }

      if (!cancelled) {
        setSearchStatus(nextStatus)
        setError(serverError)
        setMatches(found)
        setLoading(false)
      }
    }

    doSearch().catch((e) => {
      if (!cancelled) {
        setError(e)
        setSearchStatus(null)
        setLoading(false)
      }
    })

    return () => { cancelled = true; controller.abort() }
  }, [q])

  // Keyboard navigation
  const onKeyDown = useCallback((e) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); setSelected((s) => Math.min(s + 1, matches.length - 1)) }
    if (e.key === 'ArrowUp') { e.preventDefault(); setSelected((s) => Math.max(s - 1, 0)) }
    if (e.key === 'Enter' && matches[selected]) { nav(matches[selected].href) }
  }, [matches, selected, nav])

  useEffect(() => {
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onKeyDown])

  // Group results by kind
  const grouped = useMemo(() => {
    const groups = {}
    for (const m of matches) {
      if (!groups[m.kind]) groups[m.kind] = []
      groups[m.kind].push(m)
    }
    return groups
  }, [matches])

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <h1 style={{ margin: 0, fontSize: 20 }}>Search results</h1>
      <p style={{ color: 'var(--text-tertiary)', fontSize: 13, margin: 0 }}>
        Query: <code style={{ fontFamily: 'var(--font-mono)', background: 'var(--surface)', padding: '2px 6px', borderRadius: 4 }}>{q}</code>
        {matches.length > 0 && <span style={{ marginLeft: 12 }}>{matches.length} result{matches.length !== 1 ? 's' : ''}</span>}
      </p>

      {loading && <SkeletonCard lines={4} />}
      {searchStatus && <EndpointStatusNotice status={searchStatus} title="Search index unavailable" />}
      {error && (
        <NoticeState
          title="Search service degraded"
          variant="error"
          description={`Server-side search returned an error. Direct block, address, validator, and package lookups are still shown below when available. ${error.message}`}
        />
      )}

      {!loading && matches.length === 0 ? (
        <EmptyState
          title="No matches"
          description={
            <div style={{ display: 'grid', gap: 8, fontSize: 13, marginTop: 8 }}>
              <p>Try searching for:</p>
              <ul style={{ margin: 0, paddingLeft: 20, display: 'grid', gap: 4, color: 'var(--text-secondary)' }}>
                <li>A <strong>block height</strong> — e.g. <code>42</code></li>
                <li>An <strong>EVM address</strong> — e.g. <code>0x1234…abcd</code></li>
                <li>A <strong>block or tx hash</strong> — e.g. <code>0xabcdef…</code> (64 hex)</li>
                <li>A <strong>package</strong> — e.g. <code>npm/express@4.18.0</code></li>
              </ul>
            </div>
          }
        />
      ) : (
        <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
          {Object.entries(grouped).map(([kind, items]) => (
            <div key={kind}>
              <div style={{ fontSize: 11, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em', marginBottom: 8 }}>
                {KIND_ICON[kind] || '•'} {kind}s ({items.length})
              </div>
              <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
                <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
                  {items.map((m, i) => {
                    const globalIdx = matches.indexOf(m)
                    const isSelected = globalIdx === selected
                    return (
                      <li key={i}>
                        <Link
                          to={m.href}
                          style={{
                            display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 12,
                            padding: 'var(--space-3) var(--space-4)',
                            borderBottom: '1px solid var(--border)',
                            textDecoration: 'none',
                            background: isSelected ? 'var(--surface-hover)' : 'transparent',
                            transition: 'background var(--transition-fast)',
                          }}
                          onMouseEnter={() => setSelected(globalIdx)}
                        >
                          <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
                            <span style={{ fontSize: 16 }}>{KIND_ICON[m.kind] || '•'}</span>
                            <div style={{ minWidth: 0 }}>
                              <div style={{ color: 'var(--accent-primary-light)', fontFamily: 'var(--font-mono)', fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                                {m.title}
                              </div>
                              {m.subtitle && (
                                <div style={{ color: 'var(--text-tertiary)', fontSize: 11, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                                  {m.subtitle}
                                </div>
                              )}
                            </div>
                          </div>
                          <StatusBadge variant={KIND_COLOR[m.kind] || 'muted'}>{m.kind}</StatusBadge>
                        </Link>
                      </li>
                    )
                  })}
                </ul>
              </div>
            </div>
          ))}
        </div>
      )}

      <p style={{ color: 'var(--text-tertiary)', fontSize: 11, marginTop: 'var(--space-4)' }}>
        Tip: Use ↑↓ arrow keys to navigate results, Enter to open.
      </p>
    </div>
  )
}
