import React, { useCallback, useMemo, useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { usePolling } from '../hooks/usePolling.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { CursorPager } from '../components/Pagination.jsx'
import { SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { formatNumber } from '../utils/format.js'

const PAGE_SIZE = 25

/**
 * /blocks — paginated block list with cursor-based navigation.
 * Uses `before_height` / `after_height` for duplicate-free pagination
 * while blocks are being produced. Falls back to offset when no cursor.
 *
 * URL state: ?before=H or ?after=H — bookmarkable and shareable.
 * Live tail: auto-scrolls to latest blocks when enabled.
 */
export default function BlockList() {
  const [params, setParams] = useSearchParams()
  const before = params.get('before')
  const after = params.get('after')
  const [autoFollow, setAutoFollow] = useState(!before && !after)

  // Build the API args — cursor mode if params present, else latest
  const apiArgs = useMemo(() => {
    if (before) return { limit: PAGE_SIZE, before: Number(before) }
    if (after) return { limit: PAGE_SIZE, after: Number(after) }
    return { limit: PAGE_SIZE }
  }, [before, after])

  const { data, error, loading, refetch } = usePolling(
    (signal) => nodeApi.blocks(apiArgs, signal),
    { intervalMs: autoFollow ? 5000 : 15000, deps: [before, after] },
  )

  const blocks = Array.isArray(data) ? data : (data?.blocks || [])
  const tipHeight = data?.tip_height
  const nextBefore = data?.next_before_height
  const nextAfter = data?.next_after_height

  // Can we go newer/older?
  const canNewer = before != null || after != null // can always go back to latest
  const canOlder = blocks.length >= PAGE_SIZE // more blocks available

  const goNewer = useCallback(() => {
    const oldest = blocks[blocks.length - 1]
    if (!oldest) { setParams({}); return }
    // Go to blocks after the current window
    const firstHeight = blocks[0]?.height
    if (firstHeight != null && tipHeight != null && firstHeight >= tipHeight) {
      // Already at tip — clear params
      setParams({})
      setAutoFollow(true)
    } else if (nextAfter != null) {
      setParams({ after: String(nextAfter) }, { replace: true })
      setAutoFollow(false)
    } else {
      setParams({})
      setAutoFollow(true)
    }
  }, [blocks, tipHeight, nextAfter, setParams])

  const goOlder = useCallback(() => {
    const oldest = blocks[blocks.length - 1]
    if (!oldest) return
    const h = oldest.height
    if (h != null) {
      setParams({ before: String(h) }, { replace: true })
      setAutoFollow(false)
    }
  }, [blocks, setParams])

  const goLatest = useCallback(() => {
    setParams({})
    setAutoFollow(true)
  }, [setParams])

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      {/* ── Header ── */}
      <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, flexWrap: 'wrap' }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 12 }}>
          <h1 style={{ margin: 0, fontSize: 20 }}>Blocks</h1>
          {tipHeight != null && (
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 12, color: 'var(--text-tertiary)' }}>
              tip: #{formatNumber(tipHeight)}
            </span>
          )}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>
            {loading && !blocks.length ? 'loading…' : `showing ${blocks.length}`}
          </span>
          <button
            type="button"
            onClick={goLatest}
            disabled={autoFollow}
            style={{
              padding: '5px 12px',
              borderRadius: 'var(--radius-sm)',
              border: `1px solid ${autoFollow ? 'var(--accent-success)' : 'var(--border)'}`,
              background: autoFollow ? 'rgba(34,197,94,0.08)' : 'var(--surface)',
              color: autoFollow ? 'var(--accent-success)' : 'var(--text-secondary)',
              fontSize: 11, fontWeight: 600, cursor: autoFollow ? 'default' : 'pointer',
              transition: 'all var(--transition-fast)',
            }}
          >
            {autoFollow ? '● Live' : '⟳ Go to latest'}
          </button>
        </div>
      </header>

      {/* ── Block table ── */}
      {error && !blocks.length ? (
        <ErrorState error={error} onRetry={refetch} title="Could not load blocks" />
      ) : (
        <div className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
          <table className="ce-table">
            <thead>
              <tr>
                <th style={{ width: 100 }}>Height</th>
                <th>Hash</th>
                <th style={{ width: 80 }}>Txs</th>
                <th style={{ width: 180 }}>Producer</th>
                <th style={{ width: 100 }}>Status</th>
                <th style={{ width: 140 }}>Age</th>
              </tr>
            </thead>
            <tbody>
              {loading && !blocks.length
                ? Array.from({ length: 10 }).map((_, i) => <SkeletonRow key={i} cells={6} />)
                : blocks.map((b) => {
                  const h = b.height
                  const isFinalized = b.finalized ?? false
                  return (
                    <tr key={h ?? b.hash}>
                      <td style={{ fontFamily: 'var(--font-mono)', fontWeight: 600 }}>
                        <Link to={`/block/${h}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>#{h}</Link>
                      </td>
                      <td><Hash value={b.hash} kind="block-hash" start={8} end={6} /></td>
                      <td style={{ color: 'var(--text-secondary)' }}>{b.tx_count ?? 0}</td>
                      <td><Hash value={b.producer} kind="validator" start={6} end={4} /></td>
                      <td>
                        <StatusBadge variant={isFinalized ? 'success' : 'info'}>
                          {isFinalized ? 'Finalized' : 'Pending'}
                        </StatusBadge>
                      </td>
                      <td><TimeAgo timestamp={b.timestamp} /></td>
                    </tr>
                  )
                })}
            </tbody>
          </table>
        </div>
      )}

      {/* ── Cursor pagination ── */}
      <CursorPager
        canNewer={canNewer}
        canOlder={canOlder}
        onNewer={goNewer}
        onOlder={goOlder}
        label="blocks"
      />

      {/* ── URL info ── */}
      {(before || after) && (
        <p style={{ color: 'var(--text-tertiary)', fontSize: 11, textAlign: 'center' }}>
          Viewing {before ? `blocks before height ${before}` : `blocks after height ${after}`}
          {' · '}
          <button type="button" onClick={goLatest} style={{ background: 'none', border: 'none', color: 'var(--accent-primary-light)', cursor: 'pointer', fontSize: 11, padding: 0, textDecoration: 'underline' }}>
            back to latest
          </button>
        </p>
      )}
    </div>
  )
}
