import React from 'react'
import { Link, NavLink } from 'react-router-dom'
import { SearchBar } from './SearchBar.jsx'
import { ThemeToggle } from './ThemeToggle.jsx'
import { ConnectionBanner } from './ConnectionBanner.jsx'

const NAV = [
  { to: '/', label: 'Dashboard', end: true },
  { to: '/blocks', label: 'Blocks' },
  { to: '/pending', label: 'Pending' },
  { to: '/validators', label: 'Validators' },
  { to: '/packages', label: 'Packages' },
  { to: '/consensus', label: 'Consensus' },
  { to: '/bridge', label: 'Bridge' },
  { to: '/network', label: 'Network' },
  { to: '/events', label: 'Events' },
  { to: '/wallet', label: 'Wallet' },
]

const navLinkStyle = ({ isActive }) => ({
  padding: '6px 12px',
  borderRadius: 'var(--radius-sm)',
  color: isActive ? 'var(--accent-primary-light)' : 'var(--text-secondary)',
  background: isActive ? 'rgba(99,102,241,0.12)' : 'transparent',
  fontSize: 13,
  fontWeight: 600,
  textDecoration: 'none',
  whiteSpace: 'nowrap',
  transition: 'all var(--transition-fast)',
})

export function Layout({ children, sseState, reconnectAttempt, chainStats }) {
  return (
    <div style={{ minHeight: '100vh', background: 'var(--bg)', color: 'var(--text-primary)' }}>
      <header style={{
        position: 'sticky',
        top: 0,
        zIndex: 'var(--z-sticky)',
        background: 'var(--bg-elevated)',
        borderBottom: '1px solid var(--border)',
        backdropFilter: 'blur(8px)',
      }}>
        <div style={{ maxWidth: 1440, margin: '0 auto', padding: '12px 24px', display: 'flex', alignItems: 'center', gap: 'var(--space-4)' }}>
          <Link to="/" style={{ display: 'flex', alignItems: 'center', gap: 8, textDecoration: 'none' }}>
            <span style={{
              width: 28, height: 28, borderRadius: 'var(--radius-sm)',
              background: 'linear-gradient(135deg, var(--accent-primary), var(--accent-info))',
              display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              color: '#fff', fontWeight: 700, fontSize: 13,
            }}>C</span>
            <span style={{ color: 'var(--text-primary)', fontWeight: 700, fontSize: 15, letterSpacing: '-0.01em' }}>
              Chain Registry
            </span>
            {chainStats?.current_height != null && (
              <span style={{ color: 'var(--text-tertiary)', fontSize: 11, fontFamily: 'var(--font-mono)', marginLeft: 8 }}>
                #{chainStats.current_height}
              </span>
            )}
          </Link>
          <SearchBar />
          <ConnectionBanner state={sseState} reconnectAttempt={reconnectAttempt} />
          <ThemeToggle />
        </div>
        <nav aria-label="Primary" style={{ maxWidth: 1440, margin: '0 auto', padding: '0 24px 10px', display: 'flex', gap: 4, overflowX: 'auto' }}>
          {NAV.map((n) => (
            <NavLink key={n.to} to={n.to} end={n.end} style={navLinkStyle}>{n.label}</NavLink>
          ))}
        </nav>
      </header>
      <main style={{ maxWidth: 1440, margin: '0 auto', padding: '24px', minHeight: 'calc(100vh - 120px)' }}>
        {children}
      </main>
      <footer style={{ maxWidth: 1440, margin: '0 auto', padding: '24px', borderTop: '1px solid var(--border)', color: 'var(--text-tertiary)', fontSize: 12, display: 'flex', justifyContent: 'space-between', gap: 16, flexWrap: 'wrap' }}>
        <span>Chain Registry Explorer · deep-linkable, keyboard-accessible, open-source</span>
        <span><Link to="/about" style={{ color: 'var(--text-secondary)' }}>About</Link></span>
      </footer>
    </div>
  )
}
