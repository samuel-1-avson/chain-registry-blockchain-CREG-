import { useEffect, useState } from 'react'
import { nodeApi } from '../api/node.js'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState } from '../components/ErrorState.jsx'
import { Link } from 'react-router-dom'
import { timeAgo } from '../utils/format.js'

export default function Reorgs() {
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(null)
  const [reorgs, setReorgs] = useState([])

  useEffect(() => {
    const controller = new AbortController()
    nodeApi.reorgs(controller.signal)
      .then((data) => {
        setReorgs(data || [])
        setLoading(false)
      })
      .catch((err) => {
        if (err.name !== 'AbortError') {
          setError(err.message)
          setLoading(false)
        }
      })
    return () => controller.abort()
  }, [])

  if (loading) return <div style={{ padding: 'var(--space-6)' }}><SkeletonCard lines={4} /></div>
  if (error) return <ErrorState error={error} />

  return (
    <div className="content-grid list-view">
      <div className="main-col panel">
        <div className="panel-header">
          <h2 className="panel-title">
            <span className="panel-icon">🔀</span>
            Chain Reorganizations
          </h2>
          <span className="panel-subtitle">Total forks detected: {reorgs.length}</span>
        </div>
        <div className="panel-content">
          {reorgs.length === 0 ? (
            <div style={{ padding: 'var(--space-6)', textAlign: 'center', color: 'var(--text-secondary)' }}>
               No chain reorganizations detected on this node yet.
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-4)' }}>
              {reorgs.map((e, idx) => (
                <div key={e.id || idx} className="event-item" style={{ border: '1px solid var(--border-warning)', background: 'rgba(245, 158, 11, 0.05)' }}>
                  <div className="event-icon" style={{ background: 'var(--accent-warning)', color: '#000' }}>
                    🔀
                  </div>
                  <div className="event-content">
                    <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                       <span className="event-title" style={{ color: 'var(--accent-warning)', fontWeight: 'bold' }}>
                         Deep Reorg Detected (Depth {e.depth})
                       </span>
                       <span className="event-time" title={e.timestamp}>{timeAgo(e.timestamp)}</span>
                    </div>
                    <div className="event-description">
                      New tip hash: <Link to={`/blocks/hash/${e.new_tip}`}>{e.new_tip.slice(0, 16)}...</Link>
                    </div>
                    {e.abandoned_blocks?.length > 0 && (
                      <div style={{ marginTop: 8, fontSize: 13, background: 'var(--surface-active)', padding: 8, borderRadius: 4 }}>
                         <strong>{e.abandoned_blocks.length} Abandoned Blocks:</strong>
                         <ul style={{ listStyle: 'none', margin: '4px 0 0 0', padding: 0 }}>
                            {e.abandoned_blocks.map(hash => (
                               <li key={hash}><Link to={`/blocks/hash/${hash}`} style={{ color: 'var(--text-disabled)' }}>{hash.slice(0, 16)}...</Link></li>
                            ))}
                         </ul>
                      </div>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
