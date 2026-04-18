import React, { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { classifySearch } from '../utils/format.js'

/**
 * Global header search. Smart-classifies input on submit:
 *   digits → /block/:height
 *   0x + 40 hex → /address/:addr
 *   0x + 64 hex → /search?q=… (server disambiguates block-hash vs tx-hash)
 *   contains @ → /package/:canonical
 *   otherwise → /search?q=…
 */
export function SearchBar({ autoFocus = false, placeholder = 'Search block / tx / address / package…' }) {
  const [q, setQ] = useState('')
  const nav = useNavigate()

  const onSubmit = (e) => {
    e.preventDefault()
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

  return (
    <form onSubmit={onSubmit} role="search" style={{ flex: 1, maxWidth: 640, position: 'relative' }}>
      <span style={{
        position: 'absolute', left: 12, top: '50%', transform: 'translateY(-50%)',
        color: 'var(--text-tertiary)', pointerEvents: 'none', fontSize: 14,
      }} aria-hidden="true">⌕</span>
      <input
        type="search"
        autoFocus={autoFocus}
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder={placeholder}
        aria-label="Search"
        style={{
          width: '100%',
          padding: '10px 14px 10px 36px',
          background: 'var(--surface)',
          border: '1px solid var(--border)',
          borderRadius: 'var(--radius-md)',
          color: 'var(--text-primary)',
          fontSize: 13,
          fontFamily: 'var(--font-sans)',
          outline: 'none',
          transition: 'border-color var(--transition-fast), box-shadow var(--transition-fast)',
        }}
        onFocus={(e) => {
          e.target.style.borderColor = 'var(--border-accent)'
          e.target.style.boxShadow = '0 0 0 3px rgba(99,102,241,0.15)'
        }}
        onBlur={(e) => {
          e.target.style.borderColor = 'var(--border)'
          e.target.style.boxShadow = 'none'
        }}
      />
    </form>
  )
}
