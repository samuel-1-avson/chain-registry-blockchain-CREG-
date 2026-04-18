import React, { useMemo, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard, SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ShareButton } from '../components/ShareButton.jsx'
import { CursorPager } from '../components/Pagination.jsx'
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

const ACTIVITY_KINDS = ['all', 'propose', 'publish', 'revoke', 'slash', 'validator-join', 'validator-leave', 'rotate-key']

/* ── Role badges ───────────────────────────────────────────────────────────── */
function RoleBadges({ profile, txList }) {
  const roles = []
  if (profile.is_validator || profile.is_active_validator) roles.push({ label: 'Validator', variant: 'success' })
  // Check for publisher activity
  const hasPublish = txList.some((t) => t.kind === 'publish' || t.kind === 'rotate-key')
  if (hasPublish) roles.push({ label: 'Publisher', variant: 'info' })
  const hasGovernance = txList.some((t) => t.kind === 'governance-vote' || t.kind === 'propose')
  if (hasGovernance) roles.push({ label: 'Governance voter', variant: 'warning' })
  if (roles.length === 0) roles.push({ label: 'Account', variant: 'muted' })

  return (
    <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
      {roles.map((r) => (
        <StatusBadge key={r.label} variant={r.variant}>{r.label}</StatusBadge>
      ))}
    </div>
  )
}

/* ── L1 Etherscan link ─────────────────────────────────────────────────────── */
function EtherscanLink({ address }) {
  const base = import.meta.env.VITE_L1_EXPLORER_URL || 'https://etherscan.io'
  return (
    <a
      href={`${base}/address/${address}`}
      target="_blank"
      rel="noopener noreferrer"
      style={{
        display: 'inline-flex', alignItems: 'center', gap: 4,
        padding: '4px 10px', borderRadius: 'var(--radius-sm)',
        border: '1px solid var(--border)', background: 'var(--surface)',
        color: 'var(--text-secondary)', fontSize: 11, textDecoration: 'none',
        transition: 'all var(--transition-fast)',
      }}
    >
      ↗ View on L1 Explorer
    </a>
  )
}

/* ── Activity kind filter ──────────────────────────────────────────────────── */
function ActivityFilter({ active, onChange }) {
  return (
    <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
      {ACTIVITY_KINDS.map((k) => (
        <button
          key={k}
          type="button"
          onClick={() => onChange(k)}
          style={{
            padding: '3px 8px',
            borderRadius: 'var(--radius-full)',
            border: `1px solid ${active === k ? 'var(--border-accent)' : 'var(--border)'}`,
            background: active === k ? 'rgba(99,102,241,0.12)' : 'transparent',
            color: active === k ? 'var(--accent-primary-light)' : 'var(--text-tertiary)',
            fontSize: 10, fontWeight: 600, cursor: 'pointer',
            textTransform: 'uppercase', letterSpacing: '0.03em',
            transition: 'all var(--transition-fast)',
          }}
        >
          {k === 'all' ? 'All' : k}
        </button>
      ))}
    </div>
  )
}

export default function AddressPage() {
  const { addr, pubkey } = useParams()
  const raw = addr || pubkey || ''
  const address = raw.toLowerCase()
  const valid = isEvmAddress(address)

  const [scanLimit, setScanLimit] = useState(500)
  const [kindFilter, setKindFilter] = useState('all')

  const profile = useFetch((s) => nodeApi.addressProfile(address, s), {
    enabled: valid,
    deps: [address],
  })
  const txs = useFetch((s) => nodeApi.addressTransactions(address, { limit: 200, scan: scanLimit }, s), {
    enabled: valid,
    deps: [address, scanLimit],
  })

  if (!valid) {
    return <EmptyState title="Invalid address" description={`"${raw}" is not a valid EVM address.`} />
  }

  const p = profile.data || {}
  const reg = p.validator || null
  const allTxList = txs.data?.transactions || []
  const txList = kindFilter === 'all' ? allTxList : allTxList.filter((t) => t.kind === kindFilter)

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      {/* ── Header ── */}
      <header>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
          <h1 style={{ margin: 0, fontSize: 18, fontFamily: 'var(--font-mono)', wordBreak: 'break-all' }}>{address}</h1>
          <div style={{ marginLeft: 'auto', display: 'flex', gap: 8, alignItems: 'center' }}>
            <EtherscanLink address={address} />
            <ShareButton />
          </div>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 8 }}>
          <RoleBadges profile={p} txList={allTxList} />
          <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>
            · scanned last {formatNumber(p.scanned_blocks ?? 0)} blocks
          </span>
        </div>
      </header>

      {profile.error && <ErrorState error={profile.error} onRetry={profile.refetch} title="Could not load address profile" />}

      {/* ── Profile card ── */}
      {profile.loading && !profile.data ? <SkeletonCard lines={6} /> : (
        <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
          <Row k="Address" v={<Hash value={address} full showCopy />} />
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
              <Row k="Node ID"         v={reg.identity?.node_id ? <Hash value={reg.identity.node_id} full showCopy /> : '—'} />
              <Row k="Ed25519 pubkey"  v={reg.identity?.ed25519_pubkey ? <Hash value={reg.identity.ed25519_pubkey} full showCopy /> : '—'} />
            </>
          )}

          {/* Stake info */}
          <div style={{ padding: 'var(--space-3)', background: 'var(--bg-elevated)', borderRadius: 'var(--radius-sm)', display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(140px, 1fr))', gap: 'var(--space-3)' }}>
            <MiniStat label="Stake" value={`${p.stake ? formatWei(p.stake) : reg ? formatWei(reg.stake) : '0'} CREG`} />
            <MiniStat label="Reputation" value={p.reputation ?? reg?.reputation ?? '—'} />
            <MiniStat label="Blocks proposed" value={formatNumber(p.blocks_proposed ?? 0)} />
            <MiniStat label="Txs (recent)" value={formatNumber(p.tx_count ?? 0)} />
          </div>

          {reg && (
            <>
              <Row k="Registration"    v={reg.status || '—'} />
              <Row k="Consensus status" v={p.active_status || '—'} />
            </>
          )}
          <Row k="Validator page" v={
            <Link to={`/validator/${address}`} style={{ color: 'var(--accent-primary-light)', fontSize: 12 }}>
              View validator detail →
            </Link>
          } />
        </section>
      )}

      {/* ── Activity table ── */}
      <section className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
        <header style={{ padding: 'var(--space-3) var(--space-4)', borderBottom: '1px solid var(--border-subtle)', display: 'flex', justifyContent: 'space-between', alignItems: 'center', flexWrap: 'wrap', gap: 8 }}>
          <h2 style={{ margin: 0, fontSize: 14 }}>Activity</h2>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <ActivityFilter active={kindFilter} onChange={setKindFilter} />
            <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{txList.length} events</span>
          </div>
        </header>

        {txs.error && !allTxList.length ? (
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
              {txs.loading && !allTxList.length
                ? Array.from({ length: 5 }).map((_, i) => <SkeletonRow key={i} cells={4} />)
                : txList.length === 0
                  ? <tr><td colSpan={4} style={{ padding: 'var(--space-6)', textAlign: 'center', color: 'var(--text-tertiary)' }}>
                      {kindFilter !== 'all' ? `No "${kindFilter}" activity found.` : 'No activity in the scan window.'}
                    </td></tr>
                  : txList.map((t, i) => (
                    <tr key={`${t.block_height}-${t.tx_index}-${i}`}>
                      <td style={{ fontFamily: 'var(--font-mono)', fontWeight: 600 }}>
                        <Link to={`/block/${t.block_height}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>#{t.block_height}</Link>
                      </td>
                      <td><StatusBadge variant={KIND_VARIANT[t.kind] || 'muted'}>{t.kind}</StatusBadge></td>
                      <td style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-secondary)', fontSize: 12 }}>
                        {t.canonical ? (
                          <Link to={`/tx/${encodeURIComponent(t.canonical)}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none' }}>{t.canonical}</Link>
                        ) : '—'}
                      </td>
                      <td><TimeAgo timestamp={t.timestamp} /></td>
                    </tr>
                  ))}
            </tbody>
          </table>
        )}
      </section>

      {/* Load more */}
      {allTxList.length >= 200 && scanLimit < 5000 && (
        <div style={{ textAlign: 'center' }}>
          <button
            type="button"
            onClick={() => setScanLimit((prev) => Math.min(5000, prev + 500))}
            style={{
              padding: '8px 20px', background: 'var(--surface)',
              border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
              color: 'var(--accent-primary-light)', fontSize: 12, fontWeight: 600, cursor: 'pointer',
            }}
          >
            Load more (scan {scanLimit + 500} blocks)
          </button>
        </div>
      )}
    </div>
  )
}

function MiniStat({ label, value }) {
  return (
    <div>
      <div style={{ fontSize: 10, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{label}</div>
      <div style={{ fontSize: 15, fontFamily: 'var(--font-mono)', color: 'var(--text-primary)', marginTop: 2 }}>{value}</div>
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
