import React from 'react'
import { Link } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { safeL1TxUrl } from '../utils/untrustedText.js'
import { formatWei } from '../utils/format.js'
import { useFetch } from '../hooks/useFetch.js'
import { SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { Hash } from '../components/Hash.jsx'
import { CopyButton } from '../components/CopyButton.jsx'
import { UntrustedText } from '../components/UntrustedText.jsx'

export default function BountyLeaderboard() {
  const { data, error, loading, refetch } = useFetch(
    (s) => nodeApi.bountyLeaderboard({ limit: 100 }, s),
    { deps: [] },
  )
  const entries = Array.isArray(data?.entries) ? data.entries : []

  if (error && !entries.length) {
    return <ErrorState error={error} onRetry={refetch} title="Could not load discovery bounties" />
  }

  return (
    <div style={{ display: 'grid', gap: 'var(--space-4)' }}>
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', flexWrap: 'wrap', gap: 12 }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>Discovery Bounty Leaderboard</h1>
        <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{entries.length} provers</span>
      </header>

      <div
        role="note"
        className="ce-card"
        style={{
          borderColor: 'var(--border-accent)',
          fontSize: 13,
          color: 'var(--text-secondary)',
        }}
      >
        Prover bonus (25%) is paid on-chain upon discovery. Committee share (75%) is shown as{' '}
        <strong>reserved, not distributed</strong> pending the <code>ValidatorRewards.sol</code> distribution upgrade (ECA-42).
      </div>

      {loading && !data ? (
        <div className="ce-card">
          <table className="ce-table">
            <tbody>
              <SkeletonRow cells={6} />
              <SkeletonRow cells={6} />
            </tbody>
          </table>
        </div>
      ) : entries.length === 0 ? (
        <EmptyState
          title="No discovery bounties recorded yet"
          description="Confirmed L1 DiscoveryBountyAwarded events linked to Class B trigger witnesses appear here."
        />
      ) : (
        <div className="ce-card" style={{ padding: 0, overflowX: 'auto' }}>
          <table className="ce-table">
            <thead>
              <tr>
                <th>Prover</th>
                <th>Awards (Matched)</th>
                <th>Prover Bonus (25% Paid)</th>
                <th>Committee Share (75% Reserved)</th>
                <th>Latest Witness</th>
                <th>L1 Tx</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((row) => {
                const award = row.latest_award || {}
                const txUrl = safeL1TxUrl(award.l1_tx_hash)
                return (
                  <tr key={`${row.l1_chain_id}:${row.prover_address}`}>
                    <td><Hash value={row.prover_address} start={8} end={6} /></td>
                    <td style={{ fontFamily: 'var(--font-mono)' }}>
                      {row.awards} ({row.matched_witnesses})
                    </td>
                    <td style={{ fontFamily: 'var(--font-mono)' }}>
                      <StatusBadge variant="success">{formatWei(row.prover_bonus_wei)} CREG</StatusBadge>
                    </td>
                    <td style={{ fontFamily: 'var(--font-mono)' }}>
                      <StatusBadge variant="warning">
                        {formatWei(row.committee_reserved_wei)} CREG · reserved, not distributed
                      </StatusBadge>
                    </td>
                    <td>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
                        {award.canonical && (
                          <Link to={`/package/${encodeURIComponent(award.canonical)}`}>
                            <UntrustedText value={award.canonical} maxGraphemes={48} />
                          </Link>
                        )}
                        {award.witness_digest && (
                          <CopyButton
                            value={`creg evidence verify-witness ${award.witness_digest}`}
                            label="verify CLI"
                            size="xs"
                          />
                        )}
                      </div>
                    </td>
                    <td>
                      {txUrl ? (
                        <a href={txUrl} target="_blank" rel="noopener noreferrer" style={{ fontFamily: 'var(--font-mono)', fontSize: 12 }}>
                          {award.l1_tx_hash.slice(0, 10)}...
                        </a>
                      ) : (
                        <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>Invalid L1 hash</span>
                      )}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
