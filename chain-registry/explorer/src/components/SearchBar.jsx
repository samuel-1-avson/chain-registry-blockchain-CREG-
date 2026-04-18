import React, { useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate, Link } from 'react-router-dom'
import { classifySearch } from '../utils/format.js'
import { nodeApi } from '../api/node.js'
import { StatusBadge } from './StatusBadge.jsx'

const KIND_ICON = { block: '⬡', tx: '⟷', address: '◉', validator: '⬢', package: '📦', publisher: '🔑' }

/**
 * Global header search. Smart-classifies input on submit:
 *   digits → /block/:height
 *   0x + 40 hex → /address/:addr
 *   0x + 64 hex → /search?q=… (server disambiguates block-hash vs tx-hash)
 *   contains @ → /package/:canonical
 *   otherwise → /search?q=…
 *
 * While typing (debounced 300ms), shows an inline preview dropdown with top 3 matches.
 * Focus with `/` or `Ctrl+K` from anywhere.
 */
export function SearchBar({ autoFocus = false, placeholder = 'Search block / tx / address / package…' }) {
  const [q, setQ] = useState('')
  const [preview, setPreview] = useState([])
  const [showPreview, setShowPreview] = useState(false)
  const [previewIdx, setPreviewIdx] = useState(-1)
  const [loadingPreview, setLoadingPreview] = useState(false)
  const nav = useNavigate()
  const inputRef = useRef(null)
  const debounceRef = useRef(null)
  const wrapperRef = useRef(null)

  // Global keyboard shortcut: `/` or `Ctrl+K` to focus
  useEffect(() => {
    const onKey = (e) => {
      if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA' || e.target.isContentEditable) return
      if (e.key === '/' || (e.key === 'k' && (e.ctrlKey || e.metaKey))) {
        e.preventDefault()
        inputRef.current?.focus()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // Close preview on outside click
  useEffect(() => {
    const onClick = (e) => {
      if (wrapperRef.current && !wrapperRef.current.contains(e.target)) {
        setShowPreview(false)
      }
    }
    document.addEventListener('mousedown', onClick)
    return () => document.removeEventListener('mousedown', onClick)
  }, [])

  // Debounced preview search
  const fetchPreview = useCallback((query) => {
    clearTimeout(debounceRef.current)
    if (!query.trim() || query.trim().length < 2) {
      setPreview([])
      setShowPreview(false)
      return
    }
    debounceRef.current = setTimeout(async () => {
      setLoadingPreview(true)
      try {
        const result = await nodeApi.search(query)
        const items = (result?.matches || []).slice(0, 5)
        // If server returned nothing, do a quick client classify
        if (items.length === 0) {
          const cls = classifySearch(query)
          if (cls.kind === 'block-height') items.push({ kind: 'block', title: `Block #${cls.value}`, href: `/block/${cls.value}` })
          if (cls.kind === 'address') items.push({ kind: 'address', title: cls.value, href: `/address/${cls.value}` })
          if (cls.kind === 'package') items.push({ kind: 'package', title: cls.value, href: `/package/${encodeURIComponent(cls.value)}` })
        }
        setPreview(items)
        setShowPreview(items.length > 0)
        setPreviewIdx(-1)
      } catch {
        setPreview([])
      } finally {
        setLoadingPreview(false)
      }
    }, 300)
  }, [])

  const onSubmit = (e) => {
    e.preventDefault()
    setShowPreview(false)
    if (previewIdx >= 0 && preview[previewIdx]) {
      nav(preview[previewIdx].href)
      setQ('')
      return
    }
    const cls = classifySearch(q)
    switch (cls.kind) {
      case 'empty': return
      case 'block-height': nav(`/block/${cls.value}`); break
      case 'address': nav(`/address/${cls.value}`); break
      case 'hash': nav(`/search?q=${encodeURIComponent(cls.value)}`); break
      case 'package': nav(`/package/${encodeURIComponent(cls.value)}`); break
      default: nav(`/search?q=${encodeURIComponent(cls.value)}`)
    }
    setQ('')
  }

  const handleKeyDown = (e) => {
    if (!showPreview || preview.length === 0) return
    if (e.key === 'ArrowDown') { e.preventDefault(); setPreviewIdx((i) => Math.min(i + 1, preview.length - 1)) }
    if (e.key === 'ArrowUp') { e.preventDefault(); setPreviewIdx((i) => Math.max(i - 1, -1)) }
    if (e.key === 'Escape') { setShowPreview(false) }
  }

  return (
    <div ref={wrapperRef} style={{ flex: 1, maxWidth: 640, position: 'relative' }}>
      <form onSubmit={onSubmit} role="search">
        <span style={{
          position: 'absolute', left: 12, top: '50%', transform: 'translateY(-50%)',
          color: 'var(--text-tertiary)', pointerEvents: 'none', fontSize: 14,
        }} aria-hidden="true">⌕</span>
        <input
          ref={inputRef}
          type="search"
          autoFocus={autoFocus}
          value={q}
          onChange={(e) => { setQ(e.target.value); fetchPreview(e.target.value) }}
          onFocus={() => { if (preview.length > 0) setShowPreview(true) }}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          aria-label="Search"
          style={{
            width: '100%',
            padding: '10px 14px 10px 36px',
            background: 'var(--surface)',
            border: '1px solid var(--border)',
            borderRadius: showPreview ? 'var(--radius-md) var(--radius-md) 0 0' : 'var(--radius-md)',
            color: 'var(--text-primary)',
            fontSize: 13,
            fontFamily: 'var(--font-sans)',
            outline: 'none',
            transition: 'border-color var(--transition-fast), box-shadow var(--transition-fast)',
          }}
        />
        {/* Keyboard hint */}
        {!q && (
          <span style={{
            position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)',
            padding: '2px 6px', borderRadius: 4,
            border: '1px solid var(--border)', background: 'var(--bg-elevated)',
            color: 'var(--text-tertiary)', fontSize: 10, fontFamily: 'var(--font-mono)',
            pointerEvents: 'none',
          }}>/</span>
        )}
        {loadingPreview && q && (
          <span style={{
            position: 'absolute', right: 12, top: '50%', transform: 'translateY(-50%)',
            color: 'var(--text-tertiary)', fontSize: 11,
          }}>…</span>
        )}
      </form>

      {/* Preview dropdown */}
      {showPreview && preview.length > 0 && (
        <div style={{
          position: 'absolute', left: 0, right: 0, top: '100%',
          background: 'var(--bg-elevated)',
          border: '1px solid var(--border)',
          borderTop: 'none',
          borderRadius: '0 0 var(--radius-md) var(--radius-md)',
          zIndex: 'var(--z-dropdown)',
          boxShadow: 'var(--shadow-md)',
          maxHeight: 300, overflowY: 'auto',
        }}>
          {preview.map((item, i) => (
            <Link
              key={i}
              to={item.href}
              onClick={() => { setShowPreview(false); setQ('') }}
              onMouseEnter={() => setPreviewIdx(i)}
              style={{
                display: 'flex', alignItems: 'center', gap: 10,
                padding: '10px 14px',
                textDecoration: 'none',
                background: i === previewIdx ? 'var(--surface-hover)' : 'transparent',
                borderBottom: '1px solid var(--border)',
                transition: 'background var(--transition-fast)',
              }}
            >
              <span style={{ fontSize: 14 }}>{KIND_ICON[item.kind] || '•'}</span>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ color: 'var(--accent-primary-light)', fontSize: 12, fontFamily: 'var(--font-mono)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {item.title}
                </div>
                {item.subtitle && (
                  <div style={{ color: 'var(--text-tertiary)', fontSize: 10, marginTop: 1 }}>{item.subtitle}</div>
                )}
              </div>
              <StatusBadge variant="muted">{item.kind}</StatusBadge>
            </Link>
          ))}
          <div style={{ padding: '6px 14px', fontSize: 10, color: 'var(--text-tertiary)', textAlign: 'center' }}>
            ↑↓ navigate · Enter select · Esc close
          </div>
        </div>
      )}
    </div>
  )
}
