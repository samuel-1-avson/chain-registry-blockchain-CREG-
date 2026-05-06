import { useEffect, useState } from 'react'
import { getEndpointStatus, nodeApi } from '../api/node.js'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { EmptyState, EndpointStatusNotice, ErrorState } from '../components/ErrorState.jsx'
import { Hash } from '../components/Hash.jsx'

export default function RichList() {
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(null)
  const [accounts, setAccounts] = useState([])
  const [endpointStatus, setEndpointStatus] = useState(null)

  useEffect(() => {
    const controller = new AbortController()
    nodeApi.richList(controller.signal)
      .then((data) => {
        setEndpointStatus(getEndpointStatus(data))
        setAccounts(Array.isArray(data) ? data : [])
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
  if (accounts.length === 0) {
    return (
      <div className="panel" style={{ padding: 'var(--space-6)' }}>
        {endpointStatus && (
          <div style={{ marginBottom: 'var(--space-4)' }}>
            <EndpointStatusNotice status={endpointStatus} title="Rich list unavailable" />
          </div>
        )}
        <EmptyState
          title={endpointStatus ? 'Rich list unavailable' : 'No staked accounts discovered'}
          description={endpointStatus
            ? 'This node does not expose rich-list data yet, so stake rankings cannot be rendered here.'
            : 'No staked accounts discovered yet.'}
        />
      </div>
    )
  }

  // Calculate percentages based on total visible stake
  const totalStake = accounts.reduce((sum, act) => sum + (act.stake || 0), 0)

  return (
    <div className="content-grid list-view">
      <div className="main-col panel">
        <div className="panel-header">
          <h2 className="panel-title">
            <span className="panel-icon">💰</span>
            Rich List
          </h2>
          <span className="panel-subtitle">Top Accounts by Stake ({accounts.length})</span>
        </div>
        <div className="panel-content" style={{ overflowX: 'auto' }}>
          {endpointStatus && (
            <div style={{ marginBottom: 'var(--space-4)' }}>
              <EndpointStatusNotice status={endpointStatus} title="Rich list unavailable" />
            </div>
          )}
          <table className="ce-table">
            <thead>
              <tr>
                <th>Rank</th>
                <th>Address</th>
                <th>Alias</th>
                <th>Status</th>
                <th style={{ textAlign: 'right' }}>Stake (CREG)</th>
                <th style={{ textAlign: 'right' }}>% Share</th>
              </tr>
            </thead>
            <tbody>
              {accounts.map((act, i) => {
                const shareStr = totalStake > 0 ? ((act.stake / totalStake) * 100).toFixed(2) : '0.00'
                const statusBadge = act.active ? 'badge-success' : 'badge-neutral'
                return (
                  <tr key={act.identity?.evm_address || `r-${i}`}>
                    <td style={{ color: 'var(--text-secondary)' }}>{i + 1}</td>
                    <td>
                      <Hash value={act.identity?.evm_address} full link />
                    </td>
                    <td>
                      {act.alias && <span style={{ color: 'var(--accent-primary-light)', fontWeight: 500 }}>{act.alias}</span>}
                    </td>
                    <td>
                      <span className={`badge ${statusBadge}`}>{act.status || 'unknown'}</span>
                    </td>
                    <td style={{ textAlign: 'right', fontWeight: 600 }}>
                      {Number(act.stake).toLocaleString()}
                    </td>
                    <td style={{ textAlign: 'right', color: 'var(--text-secondary)' }}>
                      {shareStr}%
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  )
}
