// New router-driven explorer shell.
// Owns the shared SSE feed and renders page components via react-router.
// Legacy wallet/publish logic is embedded behind route-owned pages until the
// remaining UI logic is fully extracted.

import React, { useCallback, useMemo, useRef, useState } from 'react'
import { BrowserRouter, Routes, Route } from 'react-router-dom'

import { Layout } from './components/Layout.jsx'
import { useChainStats } from './hooks/useStats.js'
import { useSse } from './hooks/useSse.js'
import { applyTheme } from './components/ThemeToggle.jsx'

import Dashboard from './pages/Dashboard.jsx'
import BlockList from './pages/BlockList.jsx'
import BlockDetail from './pages/BlockDetail.jsx'
import TxDetail from './pages/TxDetail.jsx'
import AddressPage from './pages/AddressPage.jsx'
import ValidatorList from './pages/ValidatorList.jsx'
import ValidatorDetail from './pages/ValidatorDetail.jsx'
import PackageList from './pages/PackageList.jsx'
import PackageDetail from './pages/PackageDetail.jsx'
import Pending from './pages/Pending.jsx'
import Consensus from './pages/Consensus.jsx'
import EventsFeed from './pages/EventsFeed.jsx'
import Network from './pages/Network.jsx'
import Bridge from './pages/Bridge.jsx'
import Search from './pages/Search.jsx'
import Governance from './pages/Governance.jsx'
import Metrics from './pages/Metrics.jsx'
import ProofVerifier from './pages/ProofVerifier.jsx'
import RichList from './pages/RichList.jsx'
import Reorgs from './pages/Reorgs.jsx'
import About from './pages/About.jsx'
import NotFound from './pages/NotFound.jsx'
import WalletPage from './pages/WalletPage.jsx'
import PublisherDashboard from './pages/PublisherDashboard.jsx'
import PublisherProfile from './pages/PublisherProfile.jsx'

const EVENT_BUFFER_CAP = 500

class ErrorBoundary extends React.Component {
  constructor(props) {
    super(props)
    this.state = { err: null }
  }
  static getDerivedStateFromError(err) { return { err } }
  componentDidCatch(err, info) { console.error('Explorer crashed:', err, info) }
  render() {
    if (this.state.err) {
      return (
        <div style={{ padding: 40, color: '#f8fafc', background: '#0a0b0f', minHeight: '100vh' }}>
          <h1>Explorer crashed</h1>
          <pre style={{ color: '#ef4444', whiteSpace: 'pre-wrap' }}>{String(this.state.err?.stack || this.state.err)}</pre>
          <button onClick={() => location.reload()} style={{ marginTop: 20, padding: '8px 16px' }}>Reload</button>
        </div>
      )
    }
    return this.props.children
  }
}

// Apply theme early (before first render) so the shell never flashes dark→light.
if (typeof document !== 'undefined') {
  try {
    const stored = localStorage.getItem('chain-explorer-theme')
    applyTheme(stored === 'light' ? 'light' : 'dark')
  } catch {
    applyTheme('dark')
  }
}

function ExplorerShell() {
  const stats = useChainStats(4000)
  const [events, setEvents] = useState([])
  const eventsRef = useRef(events)
  eventsRef.current = events

  const handleEvent = useCallback((payload) => {
    if (!payload || typeof payload !== 'object') return
    const stamped = { ...payload, ts: Date.now(), ...(payload.ts ? {} : {}) }
    setEvents((prev) => {
      const next = [stamped, ...prev]
      if (next.length > EVENT_BUFFER_CAP) next.length = EVENT_BUFFER_CAP
      return next
    })
  }, [])

  const sse = useSse({ onEvent: handleEvent })

  const layoutProps = useMemo(() => ({
    sseState: sse.state,
    reconnectAttempt: sse.reconnectAttempt,
    chainStats: stats.data,
  }), [sse.state, sse.reconnectAttempt, stats.data])

  return (
    <Routes>
      <Route path="*" element={
        <Layout {...layoutProps}>
          <Routes>
            <Route path="/" element={<Dashboard recentEvents={events} />} />
            <Route path="/blocks" element={<BlockList />} />
            <Route path="/block/:id" element={<BlockDetail />} />
            <Route path="/block/hash/:id" element={<BlockDetail />} />
            <Route path="/tx/:id" element={<TxDetail />} />
            <Route path="/address/:addr" element={<AddressPage />} />
            <Route path="/validator/:addr" element={<ValidatorDetail />} />
            <Route path="/validators" element={<ValidatorList />} />
            <Route path="/packages" element={<PackageList />} />
            <Route path="/package/:id" element={<PackageDetail />} />
            <Route path="/wallet" element={<WalletPage />} />
            <Route path="/publisher" element={<PublisherDashboard />} />
            <Route path="/publisher/:pubkey" element={<PublisherProfile />} />
            <Route path="/pending" element={<Pending />} />
            <Route path="/consensus" element={<Consensus />} />
            <Route path="/events" element={<EventsFeed events={events} />} />
            <Route path="/network" element={<Network />} />
            <Route path="/bridge" element={<Bridge />} />
            <Route path="/governance" element={<Governance />} />
            <Route path="/metrics" element={<Metrics />} />
            <Route path="/proof" element={<ProofVerifier />} />
            <Route path="/richlist" element={<RichList />} />
            <Route path="/reorgs" element={<Reorgs />} />
            <Route path="/search" element={<Search />} />
            <Route path="/about" element={<About />} />
            <Route path="*" element={<NotFound />} />
          </Routes>
        </Layout>
      } />
    </Routes>
  )
}

export default function App() {
  return (
    <ErrorBoundary>
      <BrowserRouter>
        <ExplorerShell />
      </BrowserRouter>
    </ErrorBoundary>
  )
}
