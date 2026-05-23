import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { EmptyState } from '../components/ErrorState.jsx'
import { ShareButton } from '../components/ShareButton.jsx'

const KIND_COLOR = {
  block: 'info',
  new_block: 'info',
  'consensus-vote': 'warning',
  vote: 'warning',
  tx: 'success',
  publish: 'success',
  package: 'success',
  revoke: 'error',
  bridge: 'muted',
  anchor: 'muted',
  slash: 'error',
  'validator-join': 'info',
  'validator-leave': 'warning',
}

/**
 * Enhanced live events viewer backed by the shared SSE buffer from App.jsx.
 * Features:
 *  - Type filter (dropdown + chips)
 *  - Substring search filter
 *  - Auto-scroll toggle (tail mode)
 *  - Pause/resume
 *  - JSON/compact view toggle
 *  - Export to JSON
 */
export default function EventsFeed({ events = [] }) {
  const [filter, setFilter] = useState('')
  const [kind, setKind] = useState('all')
  const [autoScroll, setAutoScroll] = useState(true)
  const [paused, setPaused] = useState(false)
  const [viewMode, setViewMode] = useState('compact') // 'compact' | 'json'
  const listRef = useRef(null)
  const pausedEventsRef = useRef(events)

  // When paused, freeze the event list
  useEffect(() => {
    if (!paused) pausedEventsRef.current = events
  }, [events, paused])

  const displayEvents = paused ? pausedEventsRef.current : events

  const kinds = useMemo(
    () => Array.from(new Set(displayEvents.map((e) => e.type || e.kind || 'event'))).sort(),
    [displayEvents],
  )

  const filtered = useMemo(() => {
    return displayEvents.filter((e) => {
      if (kind !== 'all' && (e.type || e.kind) !== kind) return false
      if (filter) {
        const s = JSON.stringify(e).toLowerCase()
        if (!s.includes(filter.toLowerCase())) return false
      }
      return true
    })
  }, [displayEvents, kind, filter])

  // Auto-scroll to top on new events
  useEffect(() => {
    if (autoScroll && !paused && listRef.current) {
      listRef.current.scrollTop = 0
    }
  }, [filtered.length, autoScroll, paused])

  const exportEvents = useCallback(() => {
    const blob = new Blob([JSON.stringify(filtered, null, 2)], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `events-${new Date().toISOString().replace(/[:.]/g, '-')}.json`
    a.click()
    URL.revokeObjectURL(url)
  }, [filtered])

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      {/* Header */}
      <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, flexWrap: 'wrap' }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 12 }}>
          <h1 style={{ margin: 0, fontSize: 20 }}>Live events</h1>
          <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>
            {filtered.length} of {displayEvents.length}
          </span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <ShareButton />
        </div>
      </header>

      {/* Filter bar */}
      <div className="ce-card" style={{ display: 'flex', gap: 'var(--space-3)', flexWrap: 'wrap', alignItems: 'center' }}>
        <select
          value={kind}
          onChange={(e) => setKind(e.target.value)}
          style={{
            padding: '6px 10px',
            background: 'var(--bg-elevated)', color: 'var(--text-primary)',
            border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
            fontSize: 12,
          }}
        >
          <option value="all">All types ({displayEvents.length})</option>
          {kinds.map((k) => (
            <option key={k} value={k}>
              {k} ({displayEvents.filter((e) => (e.type || e.kind) === k).length})
            </option>
          ))}
        </select>

        <input
          type="text"
          placeholder="Filter text (substring match)…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          style={{
            flex: 1, minWidth: 200, padding: '6px 10px',
            background: 'var(--bg-elevated)', color: 'var(--text-primary)',
            border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
            fontSize: 12,
          }}
        />

        {/* Controls group */}
        <div style={{ display: 'flex', gap: 4 }}>
          <ToggleBtn
            active={!paused}
            onClick={() => setPaused((p) => !p)}
            label={paused ? '▶ Resume' : '⏸ Pause'}
            activeColor="var(--accent-success)"
          />
          <ToggleBtn
            active={autoScroll}
            onClick={() => setAutoScroll((a) => !a)}
            label={autoScroll ? '↕ Auto-scroll' : '↕ Manual'}
            activeColor="var(--accent-primary-light)"
          />
          <ToggleBtn
            active={viewMode === 'json'}
            onClick={() => setViewMode((m) => m === 'json' ? 'compact' : 'json')}
            label={viewMode === 'json' ? '{ } JSON' : '≡ Compact'}
          />
          <button type="button" onClick={exportEvents} style={btnStyle} title="Export visible events as JSON">
            ⇩ Export
          </button>
        </div>
      </div>

      {/* Kind chips */}
      {kinds.length > 1 && (
        <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
          <KindChip label="all" active={kind === 'all'} onClick={() => setKind('all')} count={displayEvents.length} />
          {kinds.map((k) => (
            <KindChip
              key={k}
              label={k}
              active={kind === k}
              onClick={() => setKind(k)}
              count={displayEvents.filter((e) => (e.type || e.kind) === k).length}
              variant={KIND_COLOR[k]}
            />
          ))}
        </div>
      )}

      {/* Events list */}
      {filtered.length === 0 ? (
        <EmptyState title="No events match" description="Relax the filter or wait for the next event." />
      ) : (
        <div
          ref={listRef}
          className="ce-card"
          style={{ padding: 0, maxHeight: 700, overflowY: 'auto' }}
        >
          {filtered.slice(0, 500).map((ev, i) => (
            <EventRow key={`${ev.ts}-${i}`} event={ev} showJson={viewMode === 'json'} />
          ))}
        </div>
      )}

      {/* Status bar */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 11, color: 'var(--text-tertiary)' }}>
        <span>
          {paused && <StatusBadge variant="warning">Paused</StatusBadge>}
          {!paused && autoScroll && <StatusBadge variant="success" pulse>Live</StatusBadge>}
          {' '}Buffer: {displayEvents.length} events
        </span>
        <span>Showing: {Math.min(filtered.length, 500)} / {filtered.length}</span>
      </div>
    </div>
  )
}

function EventRow({ event: ev, showJson }) {
  const evKind = ev.type || ev.kind || 'event'
  const variant = KIND_COLOR[evKind] || 'muted'

  return (
    <div style={{
      padding: 'var(--space-3) var(--space-4)',
      borderBottom: '1px solid var(--border)',
      fontSize: 12,
      transition: 'background var(--transition-fast)',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <StatusBadge variant={variant}>{evKind}</StatusBadge>
          {ev.height != null && (
            <Link to={`/block/${ev.height}`} style={{ color: 'var(--accent-primary-light)', fontFamily: 'var(--font-mono)', fontSize: 11, textDecoration: 'none' }}>
              #{ev.height}
            </Link>
          )}
          {ev.canonical && (
            <Link to={`/tx/${encodeURIComponent(ev.canonical)}`} style={{ color: 'var(--text-secondary)', fontFamily: 'var(--font-mono)', fontSize: 11, textDecoration: 'none' }}>
              {ev.canonical}
            </Link>
          )}
        </div>
        <TimeAgo timestamp={ev.ts || Date.now()} />
      </div>

      {showJson && (
        <pre style={{
          margin: '8px 0 0 0', fontSize: 10, color: 'var(--text-secondary)',
          whiteSpace: 'pre-wrap', wordBreak: 'break-all',
          background: 'var(--bg-elevated)', padding: 'var(--space-2)',
          borderRadius: 'var(--radius-sm)',
        }}>
          {JSON.stringify(ev, null, 2)}
        </pre>
      )}

      {!showJson && ev.message && (
        <div style={{ fontSize: 11, color: 'var(--text-secondary)', marginTop: 4 }}>
          {ev.message}
        </div>
      )}
    </div>
  )
}

function KindChip({ label, active, onClick, count, variant }) {
  const c = variant === 'success' ? 'var(--accent-success)' : variant === 'error' ? 'var(--accent-error)' : variant === 'warning' ? 'var(--accent-warning)' : variant === 'info' ? 'var(--accent-primary-light)' : 'var(--text-tertiary)'
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        padding: '3px 8px',
        borderRadius: 'var(--radius-full)',
        border: `1px solid ${active ? c : 'var(--border)'}`,
        background: active ? `${c}15` : 'transparent',
        color: active ? c : 'var(--text-tertiary)',
        fontSize: 10, fontWeight: 600, cursor: 'pointer',
        transition: 'all var(--transition-fast)',
      }}
    >
      {label} <span style={{ opacity: 0.6 }}>({count})</span>
    </button>
  )
}

function ToggleBtn({ active, onClick, label, activeColor }) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        ...btnStyle,
        border: `1px solid ${active ? (activeColor || 'var(--border-accent)') : 'var(--border)'}`,
        color: active ? (activeColor || 'var(--accent-primary-light)') : 'var(--text-tertiary)',
        background: active ? `${activeColor || 'var(--accent-primary-light)'}10` : 'var(--surface)',
      }}
    >
      {label}
    </button>
  )
}

const btnStyle = {
  padding: '5px 10px',
  borderRadius: 'var(--radius-sm)',
  border: '1px solid var(--border)',
  background: 'var(--surface)',
  color: 'var(--text-secondary)',
  fontSize: 11, fontWeight: 600, cursor: 'pointer',
  transition: 'all var(--transition-fast)',
  whiteSpace: 'nowrap',
}
