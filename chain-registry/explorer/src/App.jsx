// Chain Registry Explorer - Minimal & Intuitive Design
// Features: Clean UI, Real-time updates, Search, Animations, Detail views

import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react'

const API_BASE = 'http://127.0.0.1:8080'

// ============================================
// UTILITY FUNCTIONS
// ============================================

const formatNumber = (num) => {
  if (num >= 1e9) return (num / 1e9).toFixed(2) + 'B'
  if (num >= 1e6) return (num / 1e6).toFixed(2) + 'M'
  if (num >= 1e3) return (num / 1e3).toFixed(1) + 'k'
  return num.toString()
}

const formatStake = (val) => formatNumber(val) + ' CREG'

const timeAgo = (timestamp) => {
  if (!timestamp) return 'unknown'
  const date = new Date(timestamp)
  const seconds = Math.floor((Date.now() - date.getTime()) / 1000)
  
  if (seconds < 60) return `${seconds}s ago`
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`
  return `${Math.floor(seconds / 86400)}d ago`
}

const truncateHash = (hash, start = 8, end = 8) => {
  if (!hash || hash.length <= start + end) return hash
  return `${hash.slice(0, start)}...${hash.slice(-end)}`
}

// ============================================
// COMPONENTS
// ============================================

// Copy Button with tooltip
const CopyButton = ({ text, label }) => {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch (err) {
      console.error('Failed to copy:', err)
    }
  }

  return (
    <button className="copy-btn" onClick={handleCopy} title={`Copy ${label}`}>
      {copied ? '✓' : truncateHash(text, 6, 4)}
      <span className={`copy-tooltip ${copied ? 'show' : ''}`}>Copied!</span>
    </button>
  )
}

// Loading Skeleton
const SkeletonCard = () => (
  <div className="stat-card">
    <div className="skeleton skeleton-title" />
    <div className="skeleton skeleton-text" style={{ width: '40%' }} />
  </div>
)

// Empty State
const EmptyState = ({ icon, title, description }) => (
  <div className="empty-state">
    <div className="empty-icon">{icon}</div>
    <div className="empty-title">{title}</div>
    <div className="empty-description">{description}</div>
  </div>
)

// Status Badge
const StatusBadge = ({ status, type = 'neutral' }) => {
  const getStatusType = () => {
    if (status === 'online' || status === 'self' || status === 'active') return 'success'
    if (status === 'pending' || status === 'syncing') return 'warning'
    if (status === 'offline' || status === 'error') return 'error'
    return type
  }
  
  return <span className={`badge badge-${getStatusType()}`}>{status}</span>
}

// ============================================
// MAIN APP
// ============================================

function App() {
  // State
  const [view, setView] = useState('blocks')
  const [stats, setStats] = useState({ tip_height: 0, package_count: 0, tip_hash: '' })
  const [blocks, setBlocks] = useState([])
  const [nodes, setNodes] = useState([])
  const [p2pStatus, setP2pStatus] = useState({ peers: [], protocols: [] })
  const [bridgeStatus, setBridgeStatus] = useState({ 
    last_finalized_eth_block: 0, 
    registry_address: '', 
    bridge_sync_status: 'Initializing' 
  })
  const [events, setEvents] = useState([])
  const [selectedBlock, setSelectedBlock] = useState(null)
  const [status, setStatus] = useState('connecting')
  const [isLoading, setIsLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [isSearchFocused, setIsSearchFocused] = useState(false)
  
  const sseRef = useRef(null)
  const searchInputRef = useRef(null)

  // Fetch data
  const fetchData = useCallback(async () => {
    try {
      const [statsRes, nodesRes, p2pRes, bridgeRes] = await Promise.all([
        fetch(`${API_BASE}/v1/chain/stats`),
        fetch(`${API_BASE}/v1/nodes`),
        fetch(`${API_BASE}/v1/p2p/status`),
        fetch(`${API_BASE}/v1/bridge/status`)
      ])
      
      if (statsRes.ok) {
        const statsData = await statsRes.json()
        setStats(statsData)
        
        // Fetch blocks if we don't have them or height changed
        const currentHeight = statsData.tip_height
        if (blocks.length === 0 || currentHeight !== blocks[0]?.header?.height) {
          const blockLimit = 20
          const blockPromises = []
          for (let h = currentHeight; h >= Math.max(0, currentHeight - blockLimit); h--) {
            blockPromises.push(
              fetch(`${API_BASE}/v1/blocks/${h}`)
                .then(r => r.ok ? r.json() : null)
                .catch(() => null)
            )
          }
          const blockResults = (await Promise.all(blockPromises)).filter(b => b !== null)
          setBlocks(blockResults)
        }
      }

      if (nodesRes.ok) setNodes(await nodesRes.json())
      if (p2pRes.ok) setP2pStatus(await p2pRes.json())
      if (bridgeRes.ok) setBridgeStatus(await bridgeRes.json())
      
      setStatus('online')
      setIsLoading(false)
    } catch (err) {
      console.error('Fetch error:', err)
      setStatus('offline')
      setIsLoading(false)
    }
  }, [blocks.length])

  // Initial fetch and polling
  useEffect(() => {
    fetchData()
    const timer = setInterval(fetchData, 5000)
    return () => clearInterval(timer)
  }, [fetchData])

  // SSE Event Stream
  useEffect(() => {
    const initSSE = () => {
      const es = new EventSource(`${API_BASE}/v1/events`)
      es.onmessage = (e) => {
        try {
          const ev = JSON.parse(e.data)
          setEvents(prev => {
            const newEvents = [{ ...ev, receivedAt: Date.now() }, ...prev]
            return newEvents.slice(0, 100)
          })
        } catch (err) {}
      }
      es.onerror = () => {
        es.close()
        setTimeout(initSSE, 3000)
      }
      sseRef.current = es
    }

    initSSE()
    return () => sseRef.current?.close()
  }, [])

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e) => {
      // Search shortcut: /
      if (e.key === '/' && !isSearchFocused) {
        e.preventDefault()
        searchInputRef.current?.focus()
      }
      // Escape: clear selection
      if (e.key === 'Escape') {
        setSelectedBlock(null)
        searchInputRef.current?.blur()
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [isSearchFocused])

  // Derived state
  const totalStaked = useMemo(() => 
    nodes.reduce((acc, n) => acc + (n.stake || 0), 0),
    [nodes]
  )

  const filteredBlocks = useMemo(() => {
    if (!searchQuery) return blocks
    const query = searchQuery.toLowerCase()
    return blocks.filter(b => 
      b.header?.height?.toString().includes(query) ||
      b.hash?.toLowerCase().includes(query) ||
      b.header?.proposer_id?.toLowerCase().includes(query)
    )
  }, [blocks, searchQuery])

  // Event type classifier
  const getEventType = (eventType) => {
    if (eventType?.includes('block')) return 'block'
    if (eventType?.includes('package') || eventType?.includes('publish')) return 'package'
    if (eventType?.includes('validator')) return 'validator'
    return 'network'
  }

  const getEventIcon = (type) => {
    switch (type) {
      case 'block': return '⛓'
      case 'package': return '📦'
      case 'validator': return '⚡'
      default: return '🌐'
    }
  }

  // ============================================
  // RENDER
  // ============================================

  return (
    <div className="app-container">
      {/* Header */}
      <header className="header">
        <div className="logo">
          <div className="logo-icon">⛓</div>
          <div className="logo-text">
            <span className="logo-title">Chain Registry</span>
            <span className="logo-subtitle">Blockchain Explorer</span>
          </div>
        </div>
        
        <div className="connection-status">
          <div className={`status-dot ${status}`} />
          <span style={{ color: status === 'online' ? 'var(--accent-success)' : 'var(--accent-error)' }}>
            {status === 'online' ? 'Connected' : 'Disconnected'}
          </span>
        </div>
      </header>

      {/* Stats Grid */}
      <div className="stats-grid stagger-children">
        {isLoading ? (
          <>
            <SkeletonCard />
            <SkeletonCard />
            <SkeletonCard />
            <SkeletonCard />
          </>
        ) : (
          <>
            <div className="stat-card highlight">
              <div className="stat-header">
                <div className="stat-icon">#</div>
                <span className="stat-label">Block Height</span>
              </div>
              <div className="stat-value">{stats.tip_height.toLocaleString()}</div>
            </div>

            <div className="stat-card">
              <div className="stat-header">
                <div className="stat-icon">📦</div>
                <span className="stat-label">Packages</span>
              </div>
              <div className="stat-value">{stats.package_count.toLocaleString()}</div>
            </div>

            <div className="stat-card">
              <div className="stat-header">
                <div className="stat-icon">⚡</div>
                <span className="stat-label">Total Staked</span>
              </div>
              <div className="stat-value">{formatStake(totalStaked)}</div>
            </div>

            <div className="stat-card">
              <div className="stat-header">
                <div className="stat-icon">🌐</div>
                <span className="stat-label">Peers</span>
              </div>
              <div className="stat-value">{p2pStatus.peers.length}</div>
            </div>
          </>
        )}
      </div>

      {/* Navigation Tabs */}
      <nav className="nav-tabs">
        {[
          { id: 'blocks', label: 'Blocks', icon: '⛓' },
          { id: 'validators', label: 'Validators', icon: '⚡' },
          { id: 'p2p', label: 'Network', icon: '🌐' },
        ].map(tab => (
          <button
            key={tab.id}
            className={`nav-tab ${view === tab.id ? 'active' : ''}`}
            onClick={() => { setView(tab.id); setSelectedBlock(null) }}
          >
            <span className="nav-tab-icon">{tab.icon}</span>
            {tab.label}
          </button>
        ))}
      </nav>

      {/* Main Content */}
      <div className="content-grid">
        {/* Left Panel */}
        <div className="panel animate-fade-in">
          {/* Search Bar */}
          <div className="panel-header">
            <div className="panel-title">
              {view === 'blocks' && 'Recent Blocks'}
              {view === 'validators' && 'Validator Set'}
              {view === 'p2p' && 'Network Status'}
            </div>
            {view === 'blocks' && (
              <div className="search-box">
                <span className="search-icon">🔍</span>
                <input
                  ref={searchInputRef}
                  type="text"
                  className="search-input"
                  placeholder="Search blocks... (/)"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  onFocus={() => setIsSearchFocused(true)}
                  onBlur={() => setIsSearchFocused(false)}
                />
              </div>
            )}
          </div>

          <div className="panel-content">
            {/* Blocks View */}
            {view === 'blocks' && (
              <div className="list-container">
                {filteredBlocks.length === 0 ? (
                  <EmptyState 
                    icon="⛓" 
                    title="No blocks found" 
                    description={searchQuery ? 'Try a different search term' : 'Blocks will appear here soon'}
                  />
                ) : (
                  filteredBlocks.map((block, idx) => (
                    <div
                      key={block.header?.height || idx}
                      className={`list-item ${selectedBlock?.header?.height === block.header?.height ? 'active' : ''}`}
                      onClick={() => setSelectedBlock(block)}
                      style={{ animationDelay: `${idx * 0.05}s` }}
                    >
                      <div className="list-item-icon">#</div>
                      <div className="list-item-content">
                        <div className="list-item-title">
                          Block {block.header?.height?.toLocaleString()}
                          <span className="badge badge-neutral badge-sm">
                            {block.transactions?.length || 0} tx
                          </span>
                        </div>
                        <div className="list-item-subtitle">
                          <CopyButton text={block.hash} label="hash" />
                        </div>
                      </div>
                      <div className="list-item-meta">
                        <span className="list-item-time">{timeAgo(block.header?.timestamp)}</span>
                        <span className="badge badge-primary badge-sm">
                          {block.header?.proposer_id?.slice(0, 12)}...
                        </span>
                      </div>
                    </div>
                  ))
                )}
              </div>
            )}

            {/* Validators View */}
            {view === 'validators' && (
              <div className="table-container">
                <table className="data-table">
                  <thead>
                    <tr>
                      <th>Validator</th>
                      <th>Stake</th>
                      <th>Reputation</th>
                      <th>Status</th>
                    </tr>
                  </thead>
                  <tbody>
                    {nodes.length === 0 ? (
                      <tr>
                        <td colSpan="4">
                          <EmptyState 
                            icon="⚡" 
                            title="No validators" 
                            description="Validators will appear when the network is active"
                          />
                        </td>
                      </tr>
                    ) : (
                      nodes.map((node, idx) => (
                        <tr key={node.id} style={{ animationDelay: `${idx * 0.05}s` }} className="animate-fade-in">
                          <td>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                              <span style={{ fontWeight: 600 }}>{node.id}</span>
                              {node.alias && <span style={{ fontSize: '11px', color: 'var(--text-tertiary)' }}>{node.alias}</span>}
                            </div>
                          </td>
                          <td className="mono">{formatStake(node.stake || 0)}</td>
                          <td>
                            <div className="rep-bar">
                              <div className="rep-track">
                                <div className="rep-fill" style={{ width: `${node.reputation || 0}%` }} />
                              </div>
                              <span className="rep-value">{node.reputation || 0}</span>
                            </div>
                          </td>
                          <td><StatusBadge status={node.status} /></td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>
            )}

            {/* P2P View */}
            {view === 'p2p' && (
              <div style={{ padding: 'var(--space-4)' }}>
                <div className="detail-section">
                  <div className="detail-section-title">Connected Peers ({p2pStatus.peers.length})</div>
                  {p2pStatus.peers.length === 0 ? (
                    <EmptyState 
                      icon="🌐" 
                      title="No peers connected" 
                      description="Searching for peers via DHT..."
                    />
                  ) : (
                    <div className="peer-list">
                      {p2pStatus.peers.map((peer, idx) => (
                        <div key={idx} className="peer-item animate-slide-in" style={{ animationDelay: `${idx * 0.05}s` }}>
                          <span className="peer-id">{truncateHash(peer, 20, 8)}</span>
                          <span className="badge badge-success badge-sm">Connected</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                {p2pStatus.protocols?.length > 0 && (
                  <div className="detail-section">
                    <div className="detail-section-title">Supported Protocols</div>
                    <div className="protocol-tags">
                      {p2pStatus.protocols.map((proto, idx) => (
                        <span key={idx} className="badge badge-info">{proto}</span>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>

        {/* Right Panel - Details or Events */}
        <div className="panel animate-fade-in">
          {selectedBlock ? (
            /* Block Detail View */
            <div className="detail-panel">
              <div className="detail-header">
                <span className="detail-title">Block Details</span>
                <button className="detail-close" onClick={() => setSelectedBlock(null)}>✕</button>
              </div>
              
              <div className="detail-content">
                <div className="detail-section">
                  <div className="detail-section-title">Overview</div>
                  <div className="detail-row">
                    <span className="detail-label">Height</span>
                    <span className="detail-value">#{selectedBlock.header?.height?.toLocaleString()}</span>
                  </div>
                  <div className="detail-row">
                    <span className="detail-label">Timestamp</span>
                    <span className="detail-value">{timeAgo(selectedBlock.header?.timestamp)}</span>
                  </div>
                  <div className="detail-row">
                    <span className="detail-label">Proposer</span>
                    <span className="detail-value">{selectedBlock.header?.proposer_id}</span>
                  </div>
                  <div className="detail-row">
                    <span className="detail-label">Transactions</span>
                    <span className="detail-value">{selectedBlock.transactions?.length || 0}</span>
                  </div>
                </div>

                <div className="detail-section">
                  <div className="detail-section-title">Hashes</div>
                  <div className="detail-row">
                    <span className="detail-label">Block Hash</span>
                    <CopyButton text={selectedBlock.hash} label="hash" />
                  </div>
                  <div className="detail-row">
                    <span className="detail-label">Merkle Root</span>
                    <CopyButton text={selectedBlock.header?.merkle_root} label="root" />
                  </div>
                </div>

                {selectedBlock.transactions?.length > 0 && (
                  <div className="detail-section">
                    <div className="detail-section-title">Transactions</div>
                    {selectedBlock.transactions.map((tx, i) => (
                      <div key={i} className="tx-card">
                        <div className="tx-header">
                          <span className={`badge badge-${tx.type === 'publish' ? 'primary' : 'neutral'}`}>
                            {tx.type}
                          </span>
                          <span className="tx-id">{truncateHash(tx.id?.canonical || tx.id, 12, 4)}</span>
                        </div>
                        {tx.id?.name && (
                          <div className="tx-body">
                            <div className="tx-package">
                              {tx.id.name}
                              <span className="tx-package-version"> v{tx.id.version}</span>
                            </div>
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          ) : (
            /* Events Feed */
            <>
              <div className="panel-header">
                <div className="panel-title">
                  <span>📡</span>
                  Live Events
                  <span className="panel-subtitle">({events.length})</span>
                </div>
              </div>
              <div className="panel-content">
                <div className="list-container" style={{ maxHeight: '650px' }}>
                  {events.length === 0 ? (
                    <EmptyState 
                      icon="📡" 
                      title="No events yet" 
                      description="Events will appear here in real-time"
                    />
                  ) : (
                    events.map((ev, idx) => {
                      const eventType = getEventType(ev.event_type)
                      return (
                        <div key={idx} className="event-item animate-slide-in" style={{ animationDelay: `${idx * 0.03}s` }}>
                          <div className={`event-icon ${eventType}`}>
                            {getEventIcon(eventType)}
                          </div>
                          <div className="event-content">
                            <div className="event-title">
                              {ev.event_type?.replace(/_/g, ' ')}
                            </div>
                            <div className="event-description">
                              {ev.payload}
                            </div>
                          </div>
                          <span className="event-time">{timeAgo(ev.timestamp)}</span>
                        </div>
                      )
                    })
                  )}
                </div>
              </div>
            </>
          )}
        </div>
      </div>

      {/* Bridge HUD */}
      <div className="bridge-hud">
        <div className="bridge-header">
          <span className="bridge-icon">🌉</span>
          <div className="bridge-info">
            <div className="bridge-title">Ethereum Bridge</div>
            <div className="bridge-status">{bridgeStatus.bridge_sync_status}</div>
          </div>
          <span className="bridge-block">L1: #{bridgeStatus.last_finalized_eth_block}</span>
        </div>
        <div className="bridge-progress">
          <div 
            className="bridge-progress-fill" 
            style={{ 
              width: bridgeStatus.bridge_sync_status === 'Synced' ? '100%' : '40%',
              opacity: bridgeStatus.bridge_sync_status === 'Synced' ? 1 : 0.6
            }} 
          />
        </div>
      </div>
    </div>
  )
}

export default App
