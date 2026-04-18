import React, { useEffect, useState } from 'react'
import { timeAgo } from '../utils/format.js'

/** Relative time that self-refreshes every 15 s. Safe to mount in long lists. */
export function TimeAgo({ timestamp, absolute = false }) {
  const [, setTick] = useState(0)
  useEffect(() => {
    const id = setInterval(() => setTick((n) => n + 1), 15_000)
    return () => clearInterval(id)
  }, [])
  if (!timestamp) return <span style={{ color: 'var(--text-tertiary)' }}>—</span>
  const label = timeAgo(timestamp)
  const iso = typeof timestamp === 'number' ? new Date(timestamp).toISOString() : String(timestamp)
  return (
    <span title={iso} style={{ color: 'var(--text-secondary)', fontSize: '12px' }}>
      {absolute ? iso : label}
    </span>
  )
}
