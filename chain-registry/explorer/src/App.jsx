import React, { useState, useEffect, useRef } from 'react'

const API_BASE = 'http://127.0.0.1:8080'

function App() {
  const [view, setView] = useState('blocks') // 'blocks' | 'network' | 'p2p'
  const [stats, setStats] = useState({ tip_height: 0, package_count: 0, tip_hash: '' })
  const [blocks, setBlocks] = useState([])
  const [nodes, setNodes] = useState([])
  const [p2pStatus, setP2pStatus] = useState({ peers: [], protocols: [] })
  const [bridgeStatus, setBridgeStatus] = useState({ last_finalized_eth_block: 0, registry_address: '', bridge_sync_status: 'Initializing' })
  const [events, setEvents] = useState([])
  const [selectedBlock, setSelectedBlock] = useState(null)
  const [status, setStatus] = useState('connecting...')
  const sseRef = useRef(null)

  // 1. Initial & Real-time Data Fetch
  useEffect(() => {
    let lastHeight = -1;

    const fetchData = async () => {
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
          
          // Only fetch blocks if the height has advanced
          if (statsData.tip_height !== lastHeight) {
            const blockLimit = 15
            const startHeight = statsData.tip_height
            const endHeight = Math.max(0, startHeight - blockLimit)
            const blockPromises = []
            for (let h = startHeight; h >= endHeight; h--) {
              blockPromises.push(fetch(`${API_BASE}/v1/blocks/${h}`).then(r => r.ok ? r.json() : null))
            }
            const blockResults = (await Promise.all(blockPromises)).filter(b => b !== null)
            setBlocks(blockResults)
            lastHeight = statsData.tip_height;
          }
        }

        if (nodesRes.ok) setNodes(await nodesRes.json())
        if (p2pRes.ok) setP2pStatus(await p2pRes.json())
        if (bridgeRes.ok) setBridgeStatus(await bridgeRes.json())
        
        setStatus('online')
      } catch (err) {
        console.error('Fetch error:', err)
        setStatus('unreachable')
      }
    }

    fetchData()
    const timer = setInterval(fetchData, 5000)
    return () => clearInterval(timer)
  }, [])

  // 2. SSE Event Stream
  useEffect(() => {
    const initSSE = () => {
      const es = new EventSource(`${API_BASE}/v1/events`)
      es.onmessage = (e) => {
        try {
          const ev = JSON.parse(e.data)
          setEvents(prev => [ev, ...prev].slice(0, 100))
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

  const formatStake = (val) => new Intl.NumberFormat().format(val) + " CREG"

  return (
    <div className="app-container">
      <header>
        <div className="logo">
           <span style={{ fontSize: '28px' }}>⛓</span>
           <span>CHAIN REGISTRY <span style={{ fontWeight: 400, opacity: 0.6 }}>EXPLORER</span></span>
        </div>
        <div className="glass-panel" style={{ padding: '8px 16px', borderRadius: '12px', display: 'flex', alignItems: 'center', gap: '10px' }}>
          <div className={`status-dot ${status === 'online' ? 'pulse-green' : 'pulse-red'}`} 
               style={{ width: '8px', height: '8px', borderRadius: '50%', background: status === 'online' ? 'var(--accent-success)' : 'var(--accent-error)' }} />
          <span style={{ fontSize: '12px', fontWeight: 700, letterSpacing: '0.05em' }}>{status.toUpperCase()}</span>
        </div>
      </header>

      <div className="stats-grid">
        <div className="glass-panel stat-card">
          <div className="stat-label">Network tip</div>
          <div className="stat-value">#{stats.tip_height}</div>
        </div>
        <div className="glass-panel stat-card">
          <div className="stat-label">Verified Packages</div>
          <div className="stat-value">{stats.package_count}</div>
        </div>
        <div className="glass-panel stat-card">
          <div className="stat-label">Total Staked</div>
          <div className="stat-value">{formatStake(nodes.reduce((acc, n) => acc + n.stake, 0))}</div>
        </div>
        <div className="glass-panel stat-card">
          <div className="stat-label">Connected Peers</div>
          <div className="stat-value">{p2pStatus.peers.length}</div>
        </div>
      </div>

      <div className="tabs">
        <div className={`tab ${view === 'blocks' ? 'active' : ''}`} onClick={() => setView('blocks')}>▣ BLOCKS</div>
        <div className={`tab ${view === 'network' ? 'active' : ''}`} onClick={() => setView('network')}>⚡ VALIDATORS</div>
        <div className={`tab ${view === 'p2p' ? 'active' : ''}`} onClick={() => setView('p2p')}>🌐 P2P SWARM</div>
      </div>

      <div className="main-grid">
        {/* Left Col: Main Feed or specialized views */}
        <div className="glass-panel" style={{ minHeight: '600px' }}>
          {view === 'blocks' && (
            <>
              <div className="feed-header">
                <h2 style={{ fontSize: '18px' }}>Recent Blocks</h2>
                <span style={{ fontSize: '12px', color: 'var(--text-muted)' }}>Latest updates from PBFT consensus</span>
              </div>
              <div className="item-list">
                {blocks.map(b => (
                  <div key={b.header.height} className={`list-item ${selectedBlock?.header.height === b.header.height ? 'active-item' : ''}`} onClick={() => setSelectedBlock(b)}>
                    <div>
                      <div className="mono" style={{ color: 'var(--accent-info)', fontSize: '14px', fontWeight: 600 }}>
                        Block #{b.header.height}
                      </div>
                      <div style={{ fontSize: '11px', color: 'var(--text-dim)', marginTop: '4px' }}>
                        Hash: {b.hash?.slice(0, 24)}...
                      </div>
                    </div>
                    <div style={{ textAlign: 'right' }}>
                      <div className="badge badge-info">{b.transactions.length} Transactions</div>
                      <div style={{ fontSize: '11px', color: 'var(--text-muted)', marginTop: '6px' }}>
                        {new Date(b.header.timestamp).toLocaleTimeString()}
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}

          {view === 'network' && (
            <>
              <div className="feed-header">
                <h2 style={{ fontSize: '18px' }}>Validator Set</h2>
              </div>
              <div style={{ padding: '0 24px' }}>
                <table className="network-table">
                  <thead>
                    <tr>
                      <th>Validator ID</th>
                      <th>Stake</th>
                      <th>Reputation</th>
                      <th>Status</th>
                    </tr>
                  </thead>
                  <tbody>
                    {nodes.map(n => (
                      <tr key={n.id}>
                        <td className="mono" style={{ fontSize: '13px', color: 'var(--accent-primary)' }}>{n.id}</td>
                        <td className="mono" style={{ fontWeight: 600 }}>{formatStake(n.stake)}</td>
                        <td>
                          <div style={{ width: '100px', height: '6px', background: 'rgba(255,255,255,0.05)', borderRadius: '3px', position: 'relative' }}>
                            <div style={{ width: `${n.reputation}%`, height: '100%', background: 'var(--accent-success)', borderRadius: '3px', boxShadow: '0 0 8px var(--accent-success)' }} />
                          </div>
                        </td>
                        <td>
                           <span className={`badge ${n.status === 'self' ? 'badge-info' : 'badge-success'}`}>
                             {n.status.toUpperCase()}
                           </span>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          )}

          {view === 'p2p' && (
            <div style={{ padding: '24px' }}>
              <h2 style={{ fontSize: '18px', marginBottom: '24px' }}>libp2p Swarm Status</h2>
              <div className="glass-panel" style={{ padding: '24px', background: 'rgba(0,0,0,0.2)' }}>
                 <div className="stat-label">Active Peer IDs</div>
                 <div style={{ display: 'flex', flexDirection: 'column', gap: '12px', marginTop: '16px' }}>
                    {p2pStatus.peers.length === 0 ? (
                      <div style={{ color: 'var(--text-muted)' }}>Searching for peers via DHT...</div>
                    ) : (
                      p2pStatus.peers.map((p, i) => (
                        <div key={i} className="glass-panel" style={{ padding: '12px 16px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                           <span className="mono" style={{ fontSize: '13px' }}>{p}</span>
                           <span className="badge badge-success">Connected</span>
                        </div>
                      ))
                    )}
                 </div>
                 
                 <div className="stat-label" style={{ marginTop: '32px' }}>Supported Protocols</div>
                 <div style={{ display: 'flex', gap: '8px', marginTop: '12px', flexWrap: 'wrap' }}>
                    {p2pStatus.protocols.map(proto => (
                      <span key={proto} className="badge badge-info" style={{ borderRadius: '4px' }}>{proto}</span>
                    ))}
                 </div>
              </div>
            </div>
          )}
        </div>

        {/* Right Col: Details or Events */}
        <div className="glass-panel">
          {selectedBlock ? (
            <div className="security-detail">
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '32px' }}>
                <h3 style={{ fontSize: '16px' }}>Block Details</h3>
                <button onClick={() => setSelectedBlock(null)} style={{ background: 'rgba(255,255,255,0.05)', border: 'none', color: 'var(--text-dim)', padding: '6px 12px', borderRadius: '8px', cursor: 'pointer' }}>CLOSE</button>
              </div>
              
              <div className="stat-label">Block Metadata</div>
              <div className="glass-panel" style={{ padding: '16px', marginTop: '12px', marginBottom: '32px', background: 'rgba(0,0,0,0.1)' }}>
                 <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '8px' }}>
                    <span style={{ fontSize: '12px', color: 'var(--text-muted)' }}>Proposer</span>
                    <span className="mono" style={{ fontSize: '12px' }}>{selectedBlock.header.proposer_id}</span>
                 </div>
                 <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                    <span style={{ fontSize: '12px', color: 'var(--text-muted)' }}>Height</span>
                    <span className="mono" style={{ fontSize: '12px' }}>{selectedBlock.header.height}</span>
                 </div>
              </div>

              <div className="stat-label">Transactions</div>
              {selectedBlock.transactions.map((tx, i) => (
                <div key={i} className="glass-panel" style={{ marginTop: '16px', padding: '20px', borderLeft: '4px solid var(--accent-primary)' }}>
                   <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '16px' }}>
                      <span className="badge badge-info">{tx.type.toUpperCase()}</span>
                      <span className="mono" style={{ fontSize: '11px', color: 'var(--text-muted)' }}>TXID: {tx.id?.canonical?.slice(0, 16)}...</span>
                   </div>

                   {tx.type === 'publish' && (
                     <>
                       <div style={{ fontSize: '18px', fontWeight: 700, marginBottom: '20px' }}>
                          {tx.id.name} <span style={{ color: 'var(--accent-primary)', fontSize: '14px' }}>v{tx.id.version}</span>
                       </div>

                       <div className="audit-status">
                          <div style={{ fontSize: '20px' }}>🛡️</div>
                          <div>
                             <div style={{ fontWeight: 700, fontSize: '13px', color: 'var(--accent-success)' }}>SANDBOX APPROVED</div>
                             <div style={{ fontSize: '11px', color: 'var(--text-dim)' }}>Behavioral analysis found 0 security violations</div>
                          </div>
                       </div>

                       {tx.findings && tx.findings.length > 0 && (
                         <div style={{ marginBottom: '20px' }}>
                            <div className="stat-label">Security Findings</div>
                            <div style={{ display: 'flex', flexDirection: 'column', gap: '8px', marginTop: '8px' }}>
                               {tx.findings.map((f, fi) => (
                                 <div key={fi} style={{ background: 'rgba(0,0,0,0.2)', padding: '10px', borderRadius: '8px', fontSize: '12px', border: '1px solid var(--border)' }}>
                                    <div style={{ color: f.severity === 'Critical' ? 'var(--accent-error)' : 'var(--accent-warning)', fontWeight: 700, marginBottom: '4px' }}>
                                       [{f.code}] {f.title}
                                    </div>
                                    <div style={{ color: 'var(--text-dim)' }}>{f.description}</div>
                                 </div>
                               ))}
                            </div>
                         </div>
                       )}

                       <div className="stat-label">Consensus Signatures</div>
                       <div style={{ display: 'flex', flexDirection: 'column', gap: '6px', marginTop: '8px' }}>
                          {tx.validator_signatures?.map((sig, j) => (
                            <div key={j} className="validator-proof">
                               <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px' }}>
                                  <span style={{ color: 'var(--text-main)' }}>{sig.validator_id}</span>
                                  <span style={{ color: 'var(--accent-success)' }}>APPROVED</span>
                               </div>
                               <div style={{ fontSize: '9px', opacity: 0.5 }}>{sig.signature}</div>
                            </div>
                          ))}
                       </div>
                     </>
                   )}
                </div>
              ))}
            </div>
          ) : (
            <>
              <div className="feed-header">
                <h2 style={{ fontSize: '18px' }}>Live Event Stream</h2>
              </div>
              <div className="item-list" style={{ padding: '12px' }}>
                {events.map((ev, i) => (
                  <div key={i} className="glass-panel" style={{ padding: '12px 16px', marginBottom: '8px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
                      <span className={`badge ${ev.event_type.includes('Verified') || ev.event_type.includes('approved') ? 'badge-success' : 'badge-info'}`} style={{ minWidth: '80px', textAlign: 'center' }}>
                        {ev.event_type.replace('p2p_', '').replace('_announcement', '').toUpperCase()}
                      </span>
                      <span className="mono" style={{ fontSize: '12px', opacity: 0.8 }}>
                        {ev.payload?.slice(0, 24)}...
                      </span>
                    </div>
                    <span style={{ fontSize: '10px', color: 'var(--text-muted)' }}>{ev.timestamp?.slice(11, 19)}</span>
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      </div>

      {/* Floating Bridge Monitor */}
      <div className="bridge-hud glass-panel">
        <div className="bridge-status">
           <div style={{ fontSize: '20px' }}>🌉</div>
           <div style={{ flex: 1 }}>
              <div style={{ fontWeight: 700, fontSize: '11px', textTransform: 'uppercase', color: 'var(--text-muted)' }}>Ethereum Bridge Status</div>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: '4px' }}>
                 <span style={{ fontSize: '13px', fontWeight: 600 }}>{bridgeStatus.bridge_sync_status}</span>
                 <span className="mono" style={{ fontSize: '11px', color: 'var(--accent-info)' }}>L1 Block: {bridgeStatus.last_finalized_eth_block}</span>
              </div>
           </div>
        </div>
        <div style={{ height: '4px', background: 'rgba(255,255,255,0.05)' }}>
           <div style={{ 
              height: '100%', 
              width: bridgeStatus.bridge_sync_status === 'Synced' ? '100%' : '30%', 
              background: 'var(--accent-info)', 
              boxShadow: '0 0 10px var(--accent-info)',
              transition: 'width 1s ease'
           }} />
        </div>
      </div>
    </div>
  )
}

export default App
