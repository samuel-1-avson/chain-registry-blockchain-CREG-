import React, { useMemo } from 'react'
import { Link, useParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { TimeAgo } from '../components/TimeAgo.jsx'
import { SkeletonCard, SkeletonRow } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState, NoticeState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ShareButton } from '../components/ShareButton.jsx'
import { formatNumber, formatWei } from '../utils/format.js'

const STATUS_VARIANT = {
  verified: 'success',
  active: 'success',
  pending: 'warning',
  queued: 'warning',
  revoked: 'error',
  rejected: 'error',
}

function variantForStatus(status) {
  return STATUS_VARIANT[String(status || '').toLowerCase()] || 'muted'
}

function Stat({ label, value, hint }) {
  return (
    <div>
      <div style={{ fontSize: 10, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.05em' }}>{label}</div>
      <div style={{ fontSize: 16, fontFamily: 'var(--font-mono)', color: 'var(--text-primary)', marginTop: 2 }}>{value}</div>
      {hint && <div style={{ fontSize: 11, color: 'var(--text-tertiary)', marginTop: 2 }}>{hint}</div>}
    </div>
  )
}

function Row({ label, value }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{label}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, wordBreak: 'break-all' }}>{value ?? '—'}</span>
    </div>
  )
}

export default function PublisherProfile() {
  const { pubkey = '' } = useParams()
  const profile = useFetch((signal) => nodeApi.publisher(pubkey, signal), {
    enabled: Boolean(pubkey),
    deps: [pubkey],
  })
  const packages = useFetch((signal) => nodeApi.packages({ limit: 200, offset: 0 }, signal), {
    enabled: Boolean(pubkey),
    deps: [pubkey],
  })

  const packageItems = useMemo(() => {
    const items = packages.data?.packages || (Array.isArray(packages.data) ? packages.data : [])
    return items.filter((item) => item?.publisher === pubkey)
  }, [packages.data, pubkey])

  if (!pubkey) {
    return <EmptyState title="Missing publisher key" description="Open this page with a publisher public key." />
  }

  if (profile.error && !profile.data) {
    return <ErrorState error={profile.error} onRetry={profile.refetch} title="Could not load publisher profile" />
  }

  const data = profile.data || null
  const fetchedTotal = packages.data?.total ?? packageItems.length
  const packageCount = data?.total_packages ?? packageItems.length

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 12, flexWrap: 'wrap' }}>
        <div>
          <h1 style={{ margin: 0, fontSize: 20 }}>Publisher Profile</h1>
          <p style={{ color: 'var(--text-tertiary)', fontSize: 12, marginTop: 4 }}>
            Public-key identity, package history, and rotation-facing activity.
          </p>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <ShareButton />
        </div>
      </header>

      {profile.loading && !data ? (
        <SkeletonCard lines={6} />
      ) : data ? (
        <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-4)' }}>
          <Row label="Publisher key" value={<Hash value={data.pubkey} full showCopy />} />
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: 'var(--space-3)', padding: 'var(--space-3)', background: 'var(--bg-elevated)', borderRadius: 'var(--radius-sm)' }}>
            <Stat label="Packages" value={formatNumber(data.total_packages)} />
            <Stat label="Verified" value={formatNumber(data.verified_count)} />
            <Stat label="Revoked" value={formatNumber(data.revoked_count)} />
            <Stat label="Stake" value={`${formatWei(data.stake_wei || 0)} CREG`} />
          </div>
          <Row
            label="First seen"
            value={data.first_seen_at ? <span><TimeAgo timestamp={data.first_seen_at} />{data.first_seen_days > 0 ? ` (${data.first_seen_days} days)` : ''}</span> : '—'}
          />
          <Row
            label="Rotation surface"
            value={<StatusBadge variant={data.revoked_count > 0 ? 'warning' : 'info'}>{data.revoked_count > 0 ? 'Revocations present' : 'No revocations recorded'}</StatusBadge>}
          />
        </section>
      ) : (
        <EmptyState title="Publisher not found" description="This node does not have a profile for the requested publisher key." />
      )}

      {packages.error && !packageItems.length ? (
        <ErrorState error={packages.error} onRetry={packages.refetch} title="Could not load publisher packages" />
      ) : (
        <section className="ce-card" style={{ padding: 0, overflow: 'hidden' }}>
          <header style={{ padding: 'var(--space-3) var(--space-4)', borderBottom: '1px solid var(--border)', display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 12, flexWrap: 'wrap' }}>
            <h2 style={{ margin: 0, fontSize: 14 }}>Packages</h2>
            <span style={{ color: 'var(--text-tertiary)', fontSize: 12 }}>{formatNumber(packageCount)} total</span>
          </header>

          {packageCount > packageItems.length && fetchedTotal > packageItems.length && (
            <div style={{ padding: 'var(--space-4)' }}>
              <NoticeState
                variant="info"
                title="Showing the current fetched package window"
                description={`This node returned ${formatNumber(fetchedTotal)} packages in the current list query, and ${formatNumber(packageItems.length)} of them belong to this publisher.`}
              />
            </div>
          )}

          <table className="ce-table">
            <thead>
              <tr>
                <th>Package</th>
                <th>Version</th>
                <th>Status</th>
                <th>Published</th>
              </tr>
            </thead>
            <tbody>
              {packages.loading && !packageItems.length
                ? Array.from({ length: 5 }).map((_, index) => <SkeletonRow key={index} cells={4} />)
                : packageItems.length === 0
                  ? (
                    <tr>
                      <td colSpan={4} style={{ padding: 'var(--space-6)' }}>
                        <EmptyState title="No packages in the current list window" description="This publisher may not have published yet, or the current node package list window may not include their records." />
                      </td>
                    </tr>
                  )
                  : packageItems.map((item) => {
                    const canonical = item.canonical || `${item.ecosystem}:${item.name}`
                    return (
                      <tr key={`${canonical}:${item.version || 'unknown'}`}>
                        <td>
                          <Link to={`/package/${encodeURIComponent(canonical)}`} style={{ color: 'var(--accent-primary-light)', textDecoration: 'none', fontFamily: 'var(--font-mono)' }}>
                            {canonical}
                          </Link>
                        </td>
                        <td style={{ color: 'var(--text-secondary)', fontFamily: 'var(--font-mono)' }}>{item.version || '—'}</td>
                        <td><StatusBadge variant={variantForStatus(item.status)}>{item.status || 'unknown'}</StatusBadge></td>
                        <td><TimeAgo timestamp={item.timestamp_ms ?? item.timestamp ?? item.published_at} /></td>
                      </tr>
                    )
                  })}
            </tbody>
          </table>
        </section>
      )}
    </div>
  )
}
