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

const KIND_VARIANT = {
  publish: 'success',
  revoke: 'warning',
  slash: 'error',
  'validator-join': 'info',
  'validator-leave': 'muted',
  'rotate-key': 'info',
  propose: 'info',
}

export default function AddressPage() {
  const { addr } = useParams()
  const address = (addr || '').toLowerCase()
  const valid = isEvmAddress(address)

  const profile = useFetch((s) => nodeApi.addressProfile(address, s), {
    enabled: valid,
    deps: [address],
  })
  const txs = useFetch((s) => nodeApi.addressTransactions(address, { limit: 50 }, s), {
    enabled: valid,
    deps: [address],
  })

  if (!valid) {
    return <EmptyState title="Invalid address" description={`"${addr}" is not a valid EVM address.`} />
  }

  const p = profile.data || {}
  const reg = p.validator || null
  const txList = txs.data?.transactions || []

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header>
        <h1 style={{ margin: 0, fontSize: 18, fontFamily: 'var(--font-mono)', wordBreak: 'break-all' }}>{address}</h1>
        <p style={{ color: 'var(--text-tertiary)', fontSize: 12, marginTop: 4 }}>
          Account profile · scanned last {formatNumber(p.scanned_blocks ?? 0)} blocks
        </p>
      </header>

      {profile.error && <ErrorState error={profile.error} onRetry={profile.refetch} title="Could not load address profile" />}

      {profile.loading && !profile.data ? <SkeletonCard lines={6} /> : (
        <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
          <Row k="Address" v={<Hash value={address} full />} />
          <Row k="Validator" v={
            p.is_active_validator
              ? <StatusBadge variant="success">Active in set</StatusBadge>
              : p.is_validator
                ? <StatusBadge variant="info">Registered</StatusBadge>
                : <StatusBadge variant="muted">Not a validator</StatusBadge>
          } />
          {reg && (
            <>
              <Row k="Alias"           v={reg.alias || '—'} />
              <Row k="Node ID"         v={reg.identity?.node_id ? <Hash value={reg.identity.node_id} full /> : '—'} />
              <Row k="Ed25519 pubkey"  v={reg.identity?.ed25519_pubkey ? <Hash value={reg.identity.ed25519_pubkey} full /> : '—'} />
              <Row k="Stake"           v={(p.stake ? formatWei(p.stake) : formatWei(reg.stake)) + ' CREG'} />
              <Row k="Reputation"      v={p.reputation ?? reg.reputation ?? '—'} />
              <Row k="Registration"    v={reg.status || '—'} />
              <Row k="Consensus status" v={p.active_status || '—'} />
            </>
          )}
          <Row k="Blocks proposed"  v={<Link to={`/validator/${address}`} style={{ color: 'var(--accent-primary-light)' }}>{formatNumber(p.blocks_proposed ?? 0)}</Link>} />
          <Row k="Txs (recent)"     v={formatNumber(p.tx_count ?? 0)} />
        </section>
      )}

      <section className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
        <header style={{ padding: 'var(--space-3) var(--space-4)', borderBottom: '1px solid var(--border-subtle)', display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
          <h2 style={{ margin: 0, fontSize: 14 }}>Recent activity</h2>
          <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{txList.length} events</span>
        </header>
        {txs.error && !txList.length ? (
          <div style={{ padding: 'var(--space-4)' }}>
            <ErrorState error={txs.error} onRetry={txs.refetch} title="Could not load activity" />
          </div>
        ) : (
          <table className="ce-table">
            <thead>
              <tr>
                <th style={{ width: 90 }}>Height</th>
                <th style={{ width: 120 }}>Kind</th>
                <th>Canonical</th>
                <th style={{ width: 140 }}>Time</th>
              </tr>
            </thead>
            <tbody>
              {txs.loading && !txList.length
                ? Array.from({ length: 5 }).map((_, i) => <SkeletonRow key={i} cells={4} />)
                : txList.length === 0
                  ? <tr><td colSpan={4} style={{ padding: 'var(--space-6)', textAlign: 'center', color: 'var(--text-tertiary)' }}>No activity in the recent window.</td></tr>
                  : txList.map((t, i) => (
                    <tr key={`${t.block_height}-${t.tx_index}-${i}`}>
                      <td style={{ fontFamily: 'var(--font-mono)', fontWeight: 600 }}>
                        <Link to={`/block/${t.block_height}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>#{t.block_height}</Link>
                      </td>
                      <td><StatusBadge variant={KIND_VARIANT[t.kind] || 'muted'}>{t.kind}</StatusBadge></td>
                      <td style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-secondary)', fontSize: 12 }}>{t.canonical || '—'}</td>
                      <td><TimeAgo timestamp={t.timestamp} /></td>
                    </tr>
                  ))}
            </tbody>
          </table>
        )}
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
