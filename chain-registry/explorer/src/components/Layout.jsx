import React from 'react'
import { Link, NavLink } from 'react-router-dom'
import { SearchBar } from './SearchBar.jsx'
import { ThemeToggle } from './ThemeToggle.jsx'
import { ConnectionBanner } from './ConnectionBanner.jsx'
import { TestnetPhaseBanner } from './TestnetPhaseBanner.jsx'

const GOVERNANCE_ENABLED = import.meta.env.VITE_GOVERNANCE_ENABLED === 'true'

const NAV = [
  { to: '/', label: 'Dashboard', end: true },
  { to: '/blocks', label: 'Blocks' },
  { to: '/pending', label: 'Pending' },
  { to: '/validators', label: 'Validators' },
  { to: '/packages', label: 'Packages' },
  { to: '/consensus', label: 'Consensus' },
  { to: '/bridge', label: 'Bridge' },
  { to: '/events', label: 'Events' },
  { to: '/network', label: 'Network' },
  { to: '/metrics', label: 'Metrics' },
  ...(GOVERNANCE_ENABLED ? [{ to: '/governance', label: 'Governance' }] : []),
  { to: '/richlist', label: 'Rich List' },
  { to: '/reorgs', label: 'Reorgs' },
  { to: '/proof', label: 'Proof' },
  { to: '/wallet', label: 'Wallet' },
  { to: '/publisher', label: 'Publish' },
]

export function Layout({ children, sseState, reconnectAttempt, chainStats }) {
  return (
    <div className="explorer-shell">
      <a href="#main-content" className="explorer-skip">
        Skip to main content
      </a>
      <TestnetPhaseBanner />
      <header className="explorer-header">
        <div className="explorer-header-inner">
          <Link to="/" className="explorer-brand">
            <span className="explorer-brand-mark" aria-hidden="true">C</span>
            <span className="explorer-brand-title">Chain Registry</span>
            {chainStats?.tip_height != null && (
              <span className="explorer-brand-height">#{chainStats.tip_height}</span>
            )}
          </Link>
          <SearchBar />
          <ConnectionBanner state={sseState} reconnectAttempt={reconnectAttempt} />
          <ThemeToggle />
        </div>
        <nav aria-label="Primary" className="explorer-nav">
          {NAV.map((n) => (
            <NavLink
              key={n.to}
              to={n.to}
              end={n.end}
              className={({ isActive }) =>
                isActive ? 'explorer-nav-link active' : 'explorer-nav-link'
              }
            >
              {n.label}
            </NavLink>
          ))}
        </nav>
      </header>
      <main id="main-content" className="explorer-main">
        {children}
      </main>
      <footer className="explorer-footer">
        <span>Chain Registry Explorer — deep-linkable, keyboard-accessible, open-source</span>
        <span><Link to="/about">About</Link></span>
      </footer>
    </div>
  )
}
