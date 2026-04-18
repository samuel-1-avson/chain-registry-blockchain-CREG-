import React, { useState } from 'react'
import { Link } from 'react-router-dom'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { EmptyState } from '../components/ErrorState.jsx'

/**
 * Live events viewer backed by the shared SSE buffer maintained in App.jsx.
 * This is a filterable wrapper — the buffer itself lives upstream so the
 * connection is shared with the header's ConnectionBanner.
 */
export default function EventsFeed({ events = [] }) {
  const [filter, setFilter] = useState('')
  const [kind, setKind] = useState('all')

  const kinds = Array.from(new Set(events.map((e) => e.type || e.kind || 'event'))).sort()
  const filtered = events.filter((e) => {
    if (kind !== 'all' && (e.type || e.kind) !== kind) return false
    if (filter) {
      const s = JSON.stringify(e).toLowerCase()
      if (!s.includes(filter.toLowerCase())) return false
    }
    return true
  })

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <header style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: 16, flexWrap: 'wrap' }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>Live events</h1>
        <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{filtered.length} of {events.length}</span>
      </header>
      <div className="ce-card" style={{ display: 'flex', gap: 'var(--space-3)', flexWrap: 'wrap' }}>
        <select value={kind} onChange={(e) => setKind(e.target.value)}
          style={{ padding: '6px 10px', background: 'var(--bg-elevated)', color: 'var(--text-primary)', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', fontSize: 12 }}>
          <option value="all">All types</option>
          {kinds.map((k) => <option key={k} value={k}>{k}</option>)}
        </select>
        <input
          type="text"
          placeholder="Filter text (substring match in payload)…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          style={{ flex: 1, minWidth: 200, padding: '6px 10px', background: 'var(--bg-elevated)', color: 'var(--text-primary)', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', fontSize: 12 }}
        />
      </div>
      {filtered.length === 0 ? (
        <EmptyState title="No events match" description="Relax the filter or wait for the next event." />
      ) : (
        <div className="ce-card" style={{ padding: 0, maxHeight: 640, overflowY: 'auto' }}>
          {filtered.slice(0, 500).map((ev, i) => (
            <div key={i} style={{ padding: 'var(--space-3) var(--space-4)', borderBottom: '1px solid var(--border)', fontSize: 12 }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
                <StatusBadge variant="info">{ev.type || ev.kind || 'event'}</StatusBadge>
                <TimeAgo timestamp={ev.ts || ev.timestamp_ms || Date.now()} />
              </div>
              <pre style={{ margin: '8px 0 0 0', fontSize: 11, color: 'var(--text-secondary)', whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>
                {JSON.stringify(ev, null, 2)}
              </pre>
              {ev.height != null && <Link to={`/block/${ev.height}`} style={{ color: 'var(--accent-primary-light)', fontSize: 11 }}>Go to block #{ev.height} →</Link>}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
