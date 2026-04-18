import React from 'react'
import { Link, useParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard, SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { formatWei, formatNumber, isEvmAddress } from '../utils/format.js'

export default function ValidatorDetail() {
  const { addr } = useParams()
  const address = (addr || '').toLowerCase()
  const valid = isEvmAddress(address)

  const profile = useFetch((s) => nodeApi.validatorProfile(address, s), {
    enabled: valid,
    deps: [address],
  })

  if (!valid) {
    return <EmptyState title="Invalid address" description={`"${addr}" is not a valid EVM address.`} />
  }
  if (profile.error && !profile.data) {
    return <ErrorState error={profile.error} onRetry={profile.refetch} title="Validator not found" />
  }

  const p = profile.data || {}
  const reg = p.registration || null
  const proposals = p.recent_proposals || []

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 12 }}>
        <div>
          <h1 style={{ margin: 0, fontSize: 18, fontFamily: 'var(--font-mono)', wordBreak: 'break-all' }}>{address}</h1>
          <p style={{ color: 'var(--text-tertiary)', fontSize: 12, marginTop: 4 }}>
            Validator profile ·{' '}
            <Link to={`/address/${address}`} style={{ color: 'var(--accent-primary-light)' }}>full activity →</Link>
          </p>
        </div>
        {p.in_active_set
          ? <StatusBadge variant="success">Active in set</StatusBadge>
          : <StatusBadge variant="muted">Not in active set</StatusBadge>}
      </header>

      {profile.loading && !profile.data ? <SkeletonCard lines={6} /> : (
        <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
          <Row k="Alias"           v={reg?.alias || '—'} />
          <Row k="Stake"           v={formatWei(p.stake) + ' CREG'} />
          <Row k="Reputation"      v={formatNumber(p.reputation ?? 0)} />
          <Row k="Status"          v={<StatusBadge variant={p.status === 'online' ? 'success' : p.status === 'self' ? 'info' : 'muted'}>{p.status}</StatusBadge>} />
          {reg?.identity?.node_id && <Row k="Node ID"         v={<Hash value={reg.identity.node_id} full />} />}
          {reg?.identity?.ed25519_pubkey && <Row k="Ed25519 pubkey"  v={<Hash value={reg.identity.ed25519_pubkey} full />} />}
          {reg?.status && <Row k="Registration"    v={reg.status} />}
        </section>
      )}

      <section className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
        <header style={{ padding: 'var(--space-3) var(--space-4)', borderBottom: '1px solid var(--border-subtle)', display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
          <h2 style={{ margin: 0, fontSize: 14 }}>Recent proposals</h2>
          <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{proposals.length}</span>
        </header>
        <table className="ce-table">
          <thead>
            <tr>
              <th style={{ width: 100 }}>Height</th>
              <th>Hash</th>
              <th style={{ width: 80 }}>Txs</th>
              <th style={{ width: 140 }}>Age</th>
            </tr>
          </thead>
          <tbody>
            {profile.loading && !proposals.length
              ? Array.from({ length: 4 }).map((_, i) => <SkeletonRow key={i} cells={4} />)
              : proposals.length === 0
                ? <tr><td colSpan={4} style={{ padding: 'var(--space-6)', textAlign: 'center', color: 'var(--text-tertiary)' }}>No recent proposals in the scan window.</td></tr>
                : proposals.map((b) => (
                  <tr key={b.height ?? b.hash}>
                    <td style={{ fontFamily: 'var(--font-mono)', fontWeight: 600 }}>
                      <Link to={`/block/${b.height}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>#{b.height}</Link>
                    </td>
                    <td><Hash value={b.hash} kind="block-hash" start={8} end={6} /></td>
                    <td style={{ color: 'var(--text-secondary)' }}>{b.transactions?.length ?? b.tx_count ?? 0}</td>
                    <td><TimeAgo timestamp={b.header?.timestamp || b.timestamp_ms || b.timestamp} /></td>
                  </tr>
                ))}
          </tbody>
        </table>
      </section>
    </div>
  )
}

function Row({ k, v }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, wordBreak: 'break-all' }}>{v ?? '—'}</span>
    </div>
  )
}
