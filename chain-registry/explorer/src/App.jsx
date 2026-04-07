// Chain Registry Explorer - Public Surface
// Features: Blocks, validators, packages, wallet, staking, publish, real-time updates

import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react'
import { createPublicClient, createWalletClient, custom, http, isAddress, parseUnits, formatUnits } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'

// ============================================
// CONFIGURATION
// ============================================

const API_BASE = import.meta.env.VITE_API_BASE || ''
const FAUCET_BASE = import.meta.env.VITE_FAUCET_BASE || ''
const RPC_URL = import.meta.env.VITE_RPC_URL || (typeof window !== 'undefined' ? `${window.location.origin}/rpc` : 'http://127.0.0.1:8545')
const BUILD_CREG_TOKEN_ADDR = import.meta.env.VITE_CREG_TOKEN || null
const BUILD_STAKING_ADDR = import.meta.env.VITE_STAKING_ADDR || null
const ZERO_ADDRESS = '0x0000000000000000000000000000000000000000'

const localChain = {
  id: 31337,
  name: 'Anvil Local',
  nativeCurrency: { name: 'Ether', symbol: 'ETH', decimals: 18 },
  rpcUrls: { default: { http: [RPC_URL] } },
}

const IS_TESTNET = typeof __IS_TESTNET__ !== 'undefined' ? __IS_TESTNET__ : (import.meta.env.VITE_NETWORK || 'testnet') !== 'mainnet'

const ERC20_ABI = [
  { name: 'balanceOf', type: 'function', stateMutability: 'view',
    inputs: [{ name: 'account', type: 'address' }], outputs: [{ name: '', type: 'uint256' }] },
  { name: 'approve', type: 'function', stateMutability: 'nonpayable',
    inputs: [{ name: 'spender', type: 'address' }, { name: 'amount', type: 'uint256' }],
    outputs: [{ name: '', type: 'bool' }] },
]

const STAKING_ABI = [
  { name: 'stakeAsPublisher', type: 'function', stateMutability: 'nonpayable',
    inputs: [{ name: 'amount', type: 'uint256' }], outputs: [] },
  { name: 'applyToBeValidator', type: 'function', stateMutability: 'nonpayable',
    inputs: [{ name: 'amount', type: 'uint256' }], outputs: [] },
]

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

const normalizeContractAddress = (value) => {
  if (!value) return null
  return value.toLowerCase() === ZERO_ADDRESS ? null : value
}

const normalizeWalletAddress = (value) => {
  if (!value || typeof value !== 'string') return null
  const parts = value.split(':')
  const candidate = parts[parts.length - 1]
  return isAddress(candidate) ? candidate : null
}

const hasLeadingZeroBits = (bytes, difficulty) => {
  const fullBytes = Math.floor(difficulty / 8)
  const extraBits = difficulty % 8

  for (let index = 0; index < fullBytes; index += 1) {
    if (bytes[index] !== 0) return false
  }

  if (extraBits === 0) return true
  return (bytes[fullBytes] >> (8 - extraBits)) === 0
}

const solvePowChallenge = async (challenge, difficulty) => {
  const encoder = new TextEncoder()

  for (let attempt = 0; ; attempt += 1) {
    const nonce = attempt.toString(16)
    const payload = encoder.encode(`${challenge}${nonce}`)
    const digestBuffer = await crypto.subtle.digest('SHA-256', payload)
    const digestBytes = new Uint8Array(digestBuffer)

    if (hasLeadingZeroBits(digestBytes, difficulty)) {
      return nonce
    }

    if (attempt > 0 && attempt % 1024 === 0) {
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
  }
}

// ============================================
// ERROR BOUNDARY
// ============================================

class ErrorBoundary extends React.Component {
  constructor(props) {
    super(props)
    this.state = { hasError: false, error: null }
  }
  static getDerivedStateFromError(error) {
    return { hasError: true, error }
  }
  componentDidCatch(error, info) {
    console.error('Explorer error:', error, info)
  }
  render() {
    if (this.state.hasError) {
      return (
        <div style={{ padding: 40, textAlign: 'center', color: '#f87171', fontFamily: 'monospace' }}>
          <h2>Something went wrong</h2>
          <pre style={{ whiteSpace: 'pre-wrap', fontSize: 13 }}>{this.state.error?.message}</pre>
          <button onClick={() => window.location.reload()} style={{ marginTop: 16, padding: '8px 24px', cursor: 'pointer' }}>
            Reload
          </button>
        </div>
      )
    }
    return this.props.children
  }
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
  const [selectedValidator, setSelectedValidator] = useState(null)
  const [status, setStatus] = useState('connecting')
  const [isLoading, setIsLoading] = useState(true)
  const [fetchError, setFetchError] = useState(null)
  const [searchQuery, setSearchQuery] = useState('')
  const [isSearchFocused, setIsSearchFocused] = useState(false)

  // Wallet state
  const [walletAccount, setWalletAccount] = useState(null)
  const [walletProvider, setWalletProvider] = useState(null)
  const [walletBalance, setWalletBalance] = useState(null)
  const [walletKeyInput, setWalletKeyInput] = useState('')
  const [dripLoading, setDripLoading] = useState(false)
  const [dripResult, setDripResult] = useState(null)
  const [stakeLoading, setStakeLoading] = useState(false)
  const [stakeResult, setStakeResult] = useState(null)
  const [stakeAmount, setStakeAmount] = useState('')
  const [runtimeConfig, setRuntimeConfig] = useState({
    tokenContract: normalizeContractAddress(BUILD_CREG_TOKEN_ADDR),
    stakingContract: normalizeContractAddress(BUILD_STAKING_ADDR),
    registryAddress: null,
    isTestnet: IS_TESTNET,
  })

  // EIP-6963: multi-provider wallet discovery (W10/I1 fix)
  const [eip6963Providers, setEip6963Providers] = useState([])

  // Package state
  const [pendingPackages, setPendingPackages] = useState({ count: 0, packages: [] })
  const [packageQuery, setPackageQuery] = useState('')
  const [lookedUpPackage, setLookedUpPackage] = useState(null)
  const [packageLookupLoading, setPackageLookupLoading] = useState(false)
  const [packageList, setPackageList] = useState({ packages: [], total: 0 })
  const [packageListOffset, setPackageListOffset] = useState(0)
  const [showPublishForm, setShowPublishForm] = useState(false)
  const [publishForm, setPublishForm] = useState({ ecosystem: 'npm', name: '', version: '', ipfs_cid: '', content_hash: '', publisher_pubkey: '', signature: '' })
  const [publishStatus, setPublishStatus] = useState(null)
  const [publisherProfile, setPublisherProfile] = useState(null)
  const [publisherPackages, setPublisherPackages] = useState([])

  const sseRef = useRef(null)
  const searchInputRef = useRef(null)

  const tokenContractAddress = useMemo(
    () => normalizeContractAddress(runtimeConfig.tokenContract) || normalizeContractAddress(BUILD_CREG_TOKEN_ADDR),
    [runtimeConfig.tokenContract]
  )

  const stakingContractAddress = useMemo(
    () => normalizeContractAddress(runtimeConfig.stakingContract) || normalizeContractAddress(BUILD_STAKING_ADDR),
    [runtimeConfig.stakingContract]
  )

  // Fetch data
  const fetchData = useCallback(async () => {
    try {
      const [statsRes, nodesRes, p2pRes, bridgeRes, runtimeRes] = await Promise.all([
        fetch(`${API_BASE}/v1/chain/stats`),
        fetch(`${API_BASE}/v1/nodes`),
        fetch(`${API_BASE}/v1/p2p/status`),
        fetch(`${API_BASE}/v1/bridge/status`),
        fetch(`${API_BASE}/v1/runtime/config`).catch(() => null)
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
      if (runtimeRes?.ok) {
        const runtimeContentType = runtimeRes.headers.get('content-type') || ''
        if (runtimeContentType.includes('application/json')) {
          try {
            const runtimeData = await runtimeRes.json()
            setRuntimeConfig({
              tokenContract: normalizeContractAddress(runtimeData.token_contract),
              stakingContract: normalizeContractAddress(runtimeData.staking_contract),
              registryAddress: normalizeContractAddress(runtimeData.registry_address),
              isTestnet: runtimeData.is_testnet !== false,
            })
          } catch (runtimeError) {
            console.warn('Ignoring invalid runtime config payload:', runtimeError)
          }
        } else {
          console.warn('Ignoring non-JSON runtime config response')
        }
      }

      // Fetch pending packages
      try {
        const pendingRes = await fetch(`${API_BASE}/v1/pending`)
        if (pendingRes.ok) setPendingPackages(await pendingRes.json())
      } catch (e) { /* endpoint may not exist yet */ }

      setStatus('online')
      setFetchError(null)
      setIsLoading(false)
    } catch (err) {
      console.error('Fetch error:', err)
      setFetchError(err.message || 'Failed to connect to node')
      setStatus('offline')
      setIsLoading(false)
    }
  }, [])

  // Initial fetch and polling
  useEffect(() => {
    fetchData()
    const timer = setInterval(fetchData, 5000)
    return () => clearInterval(timer)
  }, [fetchData])

  // SSE Event Stream
  useEffect(() => {
    let retryCount = 0
    const MAX_RETRIES = 10
    let retryTimeout = null

    const initSSE = () => {
      const es = new EventSource(`${API_BASE}/v1/events`)
      es.onopen = () => { retryCount = 0 }
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
        if (retryCount < MAX_RETRIES) {
          const delay = Math.min(1000 * Math.pow(2, retryCount), 30000)
          retryCount++
          retryTimeout = setTimeout(initSSE, delay)
        }
      }
      sseRef.current = es
    }

    initSSE()
    return () => {
      sseRef.current?.close()
      if (retryTimeout) clearTimeout(retryTimeout)
    }
  }, [])

  // EIP-6963: Discover all injected wallet providers (W10 fix)
  useEffect(() => {
    const providers = []
    const handler = (event) => {
      providers.push(event.detail)
      setEip6963Providers([...providers])
    }
    window.addEventListener('eip6963:announceProvider', handler)
    // Request announcements from all injected providers
    window.dispatchEvent(new Event('eip6963:requestProvider'))
    return () => window.removeEventListener('eip6963:announceProvider', handler)
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

  const refreshWalletBalance = useCallback(async () => {
    if (!walletAccount) return

    try {
      if (tokenContractAddress) {
        const publicClient = createPublicClient({ chain: localChain, transport: http(RPC_URL) })
        const rawBalance = await publicClient.readContract({
          address: tokenContractAddress,
          abi: ERC20_ABI,
          functionName: 'balanceOf',
          args: [walletAccount.address],
        })
        setWalletBalance(formatUnits(rawBalance, 18))
        return
      }

      const res = await fetch(`${FAUCET_BASE}/api/balance/${walletAccount.address}`)
      if (res.ok) {
        const data = await res.json()
        const rawBal = data.balance || data.formatted || '0'
        try {
          setWalletBalance(formatUnits(BigInt(rawBal), 18))
        } catch {
          setWalletBalance(rawBal)
        }
      }
    } catch (e) {
      console.error('Failed to refresh wallet balance:', e)
    }
  }, [walletAccount, tokenContractAddress])

  useEffect(() => {
    if (!walletAccount) return
    refreshWalletBalance()
    const timer = setInterval(refreshWalletBalance, 10000)
    return () => clearInterval(timer)
  }, [walletAccount, refreshWalletBalance])

  const ensureWalletChain = useCallback(async (provider) => {
    if (!provider?.request) return

    const targetChainId = `0x${localChain.id.toString(16)}`
    const currentChainId = await provider.request({ method: 'eth_chainId' })
    if (currentChainId === targetChainId) return

    try {
      await provider.request({
        method: 'wallet_switchEthereumChain',
        params: [{ chainId: targetChainId }],
      })
    } catch (switchError) {
      if (switchError?.code !== 4902 && !`${switchError?.message || ''}`.includes('Unrecognized chain')) {
        throw switchError
      }

      await provider.request({
        method: 'wallet_addEthereumChain',
        params: [{
          chainId: targetChainId,
          chainName: localChain.name,
          nativeCurrency: localChain.nativeCurrency,
          rpcUrls: [RPC_URL],
          blockExplorerUrls: [window.location.origin],
        }],
      })
    }
  }, [])

  const connectExternalProvider = useCallback(async (provider, type, providerName) => {
    if (!provider?.request) {
      alert('Wallet provider unavailable.')
      return
    }

    await ensureWalletChain(provider)
    const accounts = await provider.request({ method: 'eth_requestAccounts' })
    const address = normalizeWalletAddress(accounts?.[0])

    if (!address) {
      throw new Error(`No usable account returned from ${providerName || 'wallet provider'}.`)
    }

    setWalletProvider(provider)
    setWalletAccount({ address, type, providerName })
    setDripResult(null)
    setStakeResult(null)
  }, [ensureWalletChain])

  const connectWallet = useCallback(async (privateKey) => {
    try {
      const key = privateKey.startsWith('0x') ? privateKey : `0x${privateKey}`
      const account = privateKeyToAccount(key)
      setWalletProvider(null)
      setWalletAccount({ address: account.address, type: 'local', providerName: 'Private Key', account })
      setWalletKeyInput('')
      setDripResult(null)
      setStakeResult(null)
    } catch (err) {
      alert('Invalid private key. Must be a valid 32-byte hex string.')
    }
  }, [])

  const connectMetaMask = useCallback(async () => {
    try {
      if (!window.ethereum) {
        alert('MetaMask not detected. Please install the MetaMask browser extension.')
        return
      }
      await connectExternalProvider(window.ethereum, 'metamask', 'MetaMask')
    } catch (err) {
      alert('MetaMask connection failed: ' + (err.message || err))
    }
  }, [connectExternalProvider])

  // EIP-6963: Connect via a discovered provider (W10/I1 fix)
  const connectEip6963 = useCallback(async (providerDetail) => {
    try {
      await connectExternalProvider(providerDetail.provider, 'eip6963', providerDetail.info?.name || 'Wallet')
    } catch (err) {
      alert('Wallet connection failed: ' + (err.message || err))
    }
  }, [connectExternalProvider])

  // G4: WalletConnect v2 connection
  const connectWalletConnect = useCallback(async () => {
    try {
      const { EthereumProvider } = await import('@walletconnect/ethereum-provider')
      const projectId = import.meta.env.VITE_WALLETCONNECT_PROJECT_ID
      if (!projectId) {
        alert('WalletConnect requires VITE_WALLETCONNECT_PROJECT_ID env var. Get one at https://cloud.walletconnect.com')
        return
      }
      const provider = await EthereumProvider.init({
        projectId,
        chains: [localChain.id],
        rpcMap: { [localChain.id]: RPC_URL },
        showQrModal: true,
        metadata: {
          name: 'Chain Registry Explorer',
          description: 'Package registry blockchain explorer',
          url: window.location.origin,
          icons: [],
        },
      })
      await provider.enable()
      const address = normalizeWalletAddress(provider.accounts?.[0])
      if (!address) {
        alert('No accounts returned from WalletConnect.')
        return
      }
      setWalletProvider(provider)
      setWalletAccount({ address, type: 'walletconnect', providerName: 'WalletConnect' })
      setDripResult(null)
      setStakeResult(null)
    } catch (err) {
      if (err?.message?.includes('User rejected') || err?.code === 4001) {
        // User closed the modal — not an error
        return
      }
      alert('WalletConnect failed: ' + (err.message || err))
    }
  }, [])

  const disconnectWallet = useCallback(() => {
    if (walletProvider?.disconnect) {
      Promise.resolve(walletProvider.disconnect()).catch(() => {})
    }
    setWalletProvider(null)
    setWalletAccount(null)
    setWalletBalance(null)
    setDripResult(null)
    setStakeResult(null)
    setStakeAmount('')
  }, [walletProvider])

  useEffect(() => {
    if (!walletProvider?.on) return undefined

    const handleAccountsChanged = (accounts) => {
      const nextAddress = normalizeWalletAddress(accounts?.[0])
      if (!nextAddress) {
        disconnectWallet()
        return
      }
      setWalletAccount((current) => current ? { ...current, address: nextAddress } : current)
    }

    const handleDisconnect = () => disconnectWallet()

    walletProvider.on('accountsChanged', handleAccountsChanged)
    walletProvider.on('disconnect', handleDisconnect)

    return () => {
      walletProvider.removeListener?.('accountsChanged', handleAccountsChanged)
      walletProvider.removeListener?.('disconnect', handleDisconnect)
    }
  }, [walletProvider, disconnectWallet])

  const requestDrip = useCallback(async () => {
    if (!walletAccount) return
    setDripLoading(true)
    setDripResult(null)
    try {
      const dripRequest = async (payload) => {
        const res = await fetch(`${FAUCET_BASE}/api/drip`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        })
        const data = await res.json().catch(() => ({ success: false, message: res.statusText }))
        return { res, data }
      }

      let { res, data } = await dripRequest({ address: walletAccount.address })

      if (!res.ok && /proof-of-work challenge|proof-of-work nonce|unknown or expired challenge/i.test(data?.message || '')) {
        setDripResult({ success: false, message: 'Faucet challenge issued. Solving proof-of-work…' })
        const challengeRes = await fetch(`${FAUCET_BASE}/api/challenge`)
        const challengeData = await challengeRes.json()
        const nonce = await solvePowChallenge(challengeData.challenge, challengeData.difficulty)
        ;({ res, data } = await dripRequest({
          address: walletAccount.address,
          challenge: challengeData.challenge,
          nonce,
        }))
      }

      setDripResult(data)
      if (res.ok) {
        await refreshWalletBalance()
      }
    } catch (err) {
      setDripResult({ success: false, message: err.message })
    } finally {
      setDripLoading(false)
    }
  }, [walletAccount, refreshWalletBalance])

  const doStake = useCallback(async (role) => {
    if (!walletAccount || !stakeAmount) return
    if (!tokenContractAddress || !stakingContractAddress) {
      setStakeResult({ success: false, message: 'Live contract addresses are unavailable. Wait for runtime config from the node or rebuild with testnet addresses.' })
      return
    }
    setStakeLoading(true)
    setStakeResult(null)
    try {
      const walletClient = walletProvider
        ? createWalletClient({ chain: localChain, transport: custom(walletProvider) })
        : createWalletClient({ chain: localChain, transport: http(RPC_URL) })
      const publicClient = createPublicClient({ chain: localChain, transport: http(RPC_URL) })
      const amountWei = parseUnits(stakeAmount, 18)
      const walletSigner = walletAccount.account || walletAccount.address

      if (role === 'publisher') {
        const approveTx = await walletClient.writeContract({
          account: walletSigner,
          address: tokenContractAddress,
          abi: ERC20_ABI,
          functionName: 'approve',
          args: [stakingContractAddress, amountWei],
        })
        await publicClient.waitForTransactionReceipt({ hash: approveTx })
        const stakeTx = await walletClient.writeContract({
          account: walletSigner,
          address: stakingContractAddress,
          abi: STAKING_ABI,
          functionName: 'stakeAsPublisher',
          args: [amountWei],
        })
        await publicClient.waitForTransactionReceipt({ hash: stakeTx })
        setStakeResult({ success: true, message: `Staked ${stakeAmount} tCREG as publisher`, tx: stakeTx })
      } else {
        const approveTx = await walletClient.writeContract({
          account: walletSigner,
          address: tokenContractAddress,
          abi: ERC20_ABI,
          functionName: 'approve',
          args: [stakingContractAddress, amountWei],
        })
        await publicClient.waitForTransactionReceipt({ hash: approveTx })
        const stakeTx = await walletClient.writeContract({
          account: walletSigner,
          address: stakingContractAddress,
          abi: STAKING_ABI,
          functionName: 'applyToBeValidator',
          args: [amountWei],
        })
        await publicClient.waitForTransactionReceipt({ hash: stakeTx })
        setStakeResult({ success: true, message: `Applied as validator with ${stakeAmount} tCREG`, tx: stakeTx })
      }
      await refreshWalletBalance()
    } catch (err) {
      setStakeResult({ success: false, message: err.message })
    } finally {
      setStakeLoading(false)
    }
  }, [walletAccount, walletProvider, stakeAmount, tokenContractAddress, stakingContractAddress, refreshWalletBalance])

  const lookupPackage = useCallback(async () => {
    if (!packageQuery) return
    setPackageLookupLoading(true)
    try {
      const res = await fetch(`${API_BASE}/v1/packages/${encodeURIComponent(packageQuery)}`)
      if (res.ok) {
        setLookedUpPackage(await res.json())
      } else {
        setLookedUpPackage(null)
      }
    } catch { setLookedUpPackage(null) }
    finally { setPackageLookupLoading(false) }
  }, [packageQuery])

  const fetchPackageList = useCallback(async (offset = 0) => {
    try {
      const res = await fetch(`${API_BASE}/v1/packages?offset=${offset}&limit=20`)
      if (res.ok) {
        const data = await res.json()
        setPackageList(data)
        setPackageListOffset(offset)
      }
    } catch (e) { console.error('Failed to fetch package list:', e) }
  }, [])

  useEffect(() => {
    if (view === 'packages') fetchPackageList(packageListOffset)
  }, [view, fetchPackageList, packageListOffset])

  const submitPublish = useCallback(async () => {
    setPublishStatus(null)

    // E-08: Validate inputs before submission
    const { ecosystem, name, version, content_hash, ipfs_cid, publisher_pubkey, signature } = publishForm
    if (!name || !name.trim()) {
      setPublishStatus({ ok: false, msg: 'Package name is required' })
      return
    }
    if (!version || !/^\d+\.\d+\.\d+/.test(version)) {
      setPublishStatus({ ok: false, msg: 'Version must be valid semver (e.g. 1.0.0)' })
      return
    }
    if (!content_hash || !/^[a-f0-9]{64}$/i.test(content_hash)) {
      setPublishStatus({ ok: false, msg: 'Content hash must be a 64-char hex SHA-256' })
      return
    }
    if (!ipfs_cid || !/^(Qm[a-zA-Z0-9]{44}|bafy[a-zA-Z0-9]+)$/.test(ipfs_cid)) {
      setPublishStatus({ ok: false, msg: 'IPFS CID looks invalid (expected Qm... or bafy...)' })
      return
    }
    if (!publisher_pubkey || !/^[a-f0-9]{64}$/i.test(publisher_pubkey)) {
      setPublishStatus({ ok: false, msg: 'Publisher public key must be 64 hex chars' })
      return
    }
    if (!signature || signature.length < 64) {
      setPublishStatus({ ok: false, msg: 'Signature is required' })
      return
    }

    try {
      const body = {
        id: { ecosystem: publishForm.ecosystem, name: publishForm.name, version: publishForm.version },
        content_hash: publishForm.content_hash,
        ipfs_cid: publishForm.ipfs_cid,
        publisher_pubkey: publishForm.publisher_pubkey,
        signature: publishForm.signature,
        manifest: { description: '', allowed_network_hosts: [], allowed_fs_writes: [], spawns_processes: false, allowed_process_spawns: [] },
      }
      const res = await fetch(`${API_BASE}/v1/packages`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-Requested-With': 'CregExplorer' },
        body: JSON.stringify(body),
      })
      if (res.ok) {
        setPublishStatus({ ok: true, msg: 'Package submitted for validation!' })
        setShowPublishForm(false)
        setPublishForm({ ecosystem: 'npm', name: '', version: '', ipfs_cid: '', content_hash: '', publisher_pubkey: '', signature: '' })
        fetchPackageList(0)
      } else {
        const err = await res.json().catch(() => ({ error: res.statusText }))
        setPublishStatus({ ok: false, msg: err.error || 'Submission failed' })
      }
    } catch (e) {
      setPublishStatus({ ok: false, msg: e.message })
    }
  }, [publishForm, fetchPackageList])

  const fetchPublisherProfile = useCallback(async (pubkey) => {
    try {
      const [profileRes, pkgsRes] = await Promise.all([
        fetch(`${API_BASE}/v1/publishers/${encodeURIComponent(pubkey)}`),
        fetch(`${API_BASE}/v1/packages?limit=50`),
      ])
      if (profileRes.ok) {
        setPublisherProfile(await profileRes.json())
      } else {
        setPublisherProfile(null)
      }
      if (pkgsRes.ok) {
        const data = await pkgsRes.json()
        setPublisherPackages(data.packages.filter(p => p.publisher === pubkey))
      }
      setView('publisher')
    } catch (e) { console.error('Publisher profile error:', e) }
  }, [])

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
            <span className="logo-subtitle">Public Explorer + Validator Surface</span>
          </div>
        </div>
        
        <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
          {walletAccount ? (
            <button className="wallet-btn wallet-btn-connected" onClick={() => { setView('wallet'); setSelectedBlock(null); setSelectedValidator(null) }}>
              <span className="wallet-dot connected" />
              {walletAccount.address.slice(0, 6)}...{walletAccount.address.slice(-4)}
              {walletBalance && <span className="wallet-bal">{walletBalance} tCREG</span>}
            </button>
          ) : (
            <button className="wallet-btn" onClick={() => { setView('wallet'); setSelectedBlock(null); setSelectedValidator(null) }}>
              🔑 Connect Wallet
            </button>
          )}
          <div className="connection-status">
            <div className={`status-dot ${status}`} />
            <span style={{ color: status === 'online' ? 'var(--accent-success)' : 'var(--accent-error)' }}>
              {status === 'online' ? 'Connected' : 'Disconnected'}
            </span>
          </div>
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
        ) : fetchError ? (
          <div className="stat-card" style={{ gridColumn: '1 / -1', textAlign: 'center', padding: '2rem' }}>
            <div style={{ color: '#f87171', fontSize: '1.2rem', marginBottom: '0.5rem' }}>⚠ Connection Error</div>
            <div style={{ color: '#888', fontSize: '0.9rem' }}>{fetchError}</div>
            <button onClick={() => { setIsLoading(true); setFetchError(null); fetchData() }}
              style={{ marginTop: '1rem', padding: '6px 16px', cursor: 'pointer', background: 'var(--primary)', color: '#fff', border: 'none', borderRadius: '6px' }}>
              Retry
            </button>
          </div>
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
          { id: 'packages', label: 'Packages', icon: '📦' },
          { id: 'wallet', label: 'Wallet', icon: '🔑' },
          { id: 'p2p', label: 'Network', icon: '🌐' },
        ].map(tab => (
          <button
            key={tab.id}
            className={`nav-tab ${view === tab.id ? 'active' : ''}`}
            onClick={() => { setView(tab.id); setSelectedBlock(null); setSelectedValidator(null) }}
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
              {view === 'packages' && `Packages (${stats.package_count} on-chain)`}
              {view === 'wallet' && 'Wallet, Faucet, Stake'}
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
                        <tr
                          key={node.id}
                          style={{ animationDelay: `${idx * 0.05}s`, cursor: 'pointer' }}
                          className={`animate-fade-in ${selectedValidator?.id === node.id ? 'validator-row-active' : ''}`}
                          onClick={() => {
                            setSelectedValidator(node)
                            setSelectedBlock(null)
                          }}
                        >
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

            {/* Packages View */}
            {view === 'packages' && (
              <div style={{ padding: 'var(--space-4)' }}>
                <div style={{ marginBottom: 'var(--space-4)', display: 'flex', gap: 'var(--space-2)' }}>
                  <div className="search-box" style={{ flex: 1 }}>
                    <span className="search-icon">📦</span>
                    <input
                      type="text"
                      className="search-input"
                      placeholder="Lookup package by canonical name... (press Enter)"
                      value={packageQuery}
                      onChange={(e) => setPackageQuery(e.target.value)}
                      onKeyDown={(e) => e.key === 'Enter' && lookupPackage()}
                    />
                  </div>
                  <button
                    className={`nav-tab ${showPublishForm ? 'active' : ''}`}
                    onClick={() => setShowPublishForm(!showPublishForm)}
                    style={{ whiteSpace: 'nowrap' }}
                  >
                    ➕ Publish
                  </button>
                </div>

                {/* Publish Form */}
                {showPublishForm && (
                  <div className="detail-panel" style={{ marginBottom: 'var(--space-4)' }}>
                    <div className="detail-header">
                      <span className="detail-title">Publish a Package</span>
                      <button className="detail-close" onClick={() => setShowPublishForm(false)}>✕</button>
                    </div>
                    <div className="detail-content" style={{ display: 'grid', gap: 'var(--space-2)' }}>
                      <div style={{ display: 'grid', gridTemplateColumns: '1fr 2fr 1fr', gap: 'var(--space-2)' }}>
                        <select
                          className="search-input"
                          value={publishForm.ecosystem}
                          onChange={e => setPublishForm(f => ({ ...f, ecosystem: e.target.value }))}
                        >
                          <option value="npm">npm</option>
                          <option value="pypi">pypi</option>
                          <option value="cargo">cargo</option>
                          <option value="go">go</option>
                        </select>
                        <input className="search-input" placeholder="Package name" value={publishForm.name}
                          onChange={e => setPublishForm(f => ({ ...f, name: e.target.value }))} />
                        <input className="search-input" placeholder="Version (e.g. 1.0.0)" value={publishForm.version}
                          onChange={e => setPublishForm(f => ({ ...f, version: e.target.value }))} />
                      </div>
                      <input className="search-input" placeholder="IPFS CID (bafy...)" value={publishForm.ipfs_cid}
                        onChange={e => setPublishForm(f => ({ ...f, ipfs_cid: e.target.value }))} />
                      <input className="search-input" placeholder="Content hash (SHA-256 hex)" value={publishForm.content_hash}
                        onChange={e => setPublishForm(f => ({ ...f, content_hash: e.target.value }))} />
                      <input className="search-input" placeholder="Publisher public key (hex)" value={publishForm.publisher_pubkey}
                        onChange={e => setPublishForm(f => ({ ...f, publisher_pubkey: e.target.value }))} />
                      <input className="search-input" placeholder="Ed25519 signature (hex)" value={publishForm.signature}
                        onChange={e => setPublishForm(f => ({ ...f, signature: e.target.value }))} />
                      <button className="nav-tab active" onClick={submitPublish} style={{ justifySelf: 'end', padding: '8px 24px' }}>
                        Submit Package
                      </button>
                      {publishStatus && (
                        <span className={`badge ${publishStatus.ok ? 'badge-success' : 'badge-error'}`}>
                          {publishStatus.msg}
                        </span>
                      )}
                    </div>
                  </div>
                )}

                {lookedUpPackage && (
                  <div className="detail-panel" style={{ marginBottom: 'var(--space-4)' }}>
                    <div className="detail-header">
                      <span className="detail-title">📦 {lookedUpPackage.canonical}</span>
                      <button className="detail-close" onClick={() => setLookedUpPackage(null)}>✕</button>
                    </div>
                    <div className="detail-content">
                      <div className="detail-section">
                        <div className="detail-row">
                          <span className="detail-label">Status</span>
                          <StatusBadge status={lookedUpPackage.status} />
                        </div>
                        <div className="detail-row">
                          <span className="detail-label">Publisher</span>
                          <span className="detail-value" style={{ cursor: 'pointer', textDecoration: 'underline' }}
                            onClick={() => fetchPublisherProfile(lookedUpPackage.publisher)}>
                            {truncateHash(lookedUpPackage.publisher, 10, 6)}
                          </span>
                        </div>
                        <div className="detail-row">
                          <span className="detail-label">Published</span>
                          <span className="detail-value">{timeAgo(lookedUpPackage.published_at)}</span>
                        </div>
                        {lookedUpPackage.ipfs_cid && (
                          <div className="detail-row">
                            <span className="detail-label">IPFS CID</span>
                            <CopyButton text={lookedUpPackage.ipfs_cid} label="cid" />
                          </div>
                        )}
                        {lookedUpPackage.content_hash && (
                          <div className="detail-row">
                            <span className="detail-label">Content Hash</span>
                            <CopyButton text={lookedUpPackage.content_hash} label="hash" />
                          </div>
                        )}
                      </div>
                    </div>
                  </div>
                )}

                {/* On-chain Package List */}
                {packageList.packages.length > 0 && (
                  <div className="detail-section" style={{ marginBottom: 'var(--space-4)' }}>
                    <div className="detail-section-title">On-chain Packages ({packageList.total} total)</div>
                    <div className="table-container">
                      <table className="data-table">
                        <thead>
                          <tr>
                            <th>Package</th>
                            <th>Version</th>
                            <th>Status</th>
                            <th>Publisher</th>
                            <th>Published</th>
                          </tr>
                        </thead>
                        <tbody>
                          {packageList.packages.map((pkg, idx) => (
                            <tr key={pkg.canonical || idx} className="animate-slide-in" style={{ animationDelay: `${idx * 0.03}s`, cursor: 'pointer' }}
                              onClick={() => {
                                setPackageQuery(pkg.canonical)
                                // Lookup directly with the canonical name to avoid stale closure
                                fetch(`${API_BASE}/v1/packages/${encodeURIComponent(pkg.canonical)}`)
                                  .then(r => r.ok ? r.json() : null)
                                  .then(data => setLookedUpPackage(data))
                                  .catch(() => setLookedUpPackage(null))
                              }}>
                              <td style={{ fontWeight: 600 }}>{pkg.ecosystem}:{pkg.name}</td>
                              <td>{pkg.version}</td>
                              <td><StatusBadge status={pkg.status} /></td>
                              <td>{truncateHash(pkg.publisher, 8, 4)}</td>
                              <td>{timeAgo(pkg.published_at)}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                    <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: 'var(--space-2)' }}>
                      <button className="nav-tab" disabled={packageListOffset === 0}
                        onClick={() => fetchPackageList(Math.max(0, packageListOffset - 20))}>
                        ← Previous
                      </button>
                      <span style={{ color: 'var(--text-secondary)', fontSize: '0.85rem' }}>
                        Showing {packageListOffset + 1}–{Math.min(packageListOffset + 20, packageList.total)} of {packageList.total}
                      </span>
                      <button className="nav-tab" disabled={packageListOffset + 20 >= packageList.total}
                        onClick={() => fetchPackageList(packageListOffset + 20)}>
                        Next →
                      </button>
                    </div>
                  </div>
                )}

                <div className="detail-section">
                  <div className="detail-section-title">Pending Packages ({pendingPackages.count})</div>
                  {pendingPackages.packages?.length === 0 ? (
                    <EmptyState
                      icon="📦"
                      title="No pending packages"
                      description="Packages awaiting verification will appear here"
                    />
                  ) : (
                    <div className="peer-list">
                      {pendingPackages.packages?.map((pkg, idx) => (
                        <div key={idx} className="peer-item animate-slide-in" style={{ animationDelay: `${idx * 0.05}s` }}>
                          <span style={{ fontWeight: 600, color: 'var(--text-primary)' }}>{pkg}</span>
                          <span className="badge badge-warning badge-sm">Pending</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            )}

            {/* Wallet View */}
            {view === 'wallet' && (
              <div style={{ padding: 'var(--space-4)' }}>
                <div className="detail-section" style={{ marginBottom: 'var(--space-4)' }}>
                  <div className="detail-section-title">Explorer Wallet</div>
                  <div style={{ color: 'var(--text-secondary)', marginBottom: 'var(--space-3)' }}>
                    Connect a wallet, request testnet tokens, and stake directly from the browser.
                  </div>
                </div>

                <div className="detail-section" style={{ marginBottom: 'var(--space-4)' }}>
                  <div className="detail-section-title">Live Runtime Config</div>
                  <div className="detail-row">
                    <span className="detail-label">Network Mode</span>
                    <span className="detail-value">{runtimeConfig.isTestnet ? 'Testnet' : 'Mainnet'}</span>
                  </div>
                  <div className="detail-row">
                    <span className="detail-label">Registry</span>
                    {runtimeConfig.registryAddress ? (
                      <CopyButton text={runtimeConfig.registryAddress} label="registry address" />
                    ) : (
                      <span className="detail-value">Unavailable</span>
                    )}
                  </div>
                  <div className="detail-row">
                    <span className="detail-label">Token Contract</span>
                    {tokenContractAddress ? (
                      <CopyButton text={tokenContractAddress} label="token address" />
                    ) : (
                      <span className="detail-value">Unavailable</span>
                    )}
                  </div>
                  <div className="detail-row">
                    <span className="detail-label">Staking Contract</span>
                    {stakingContractAddress ? (
                      <CopyButton text={stakingContractAddress} label="staking address" />
                    ) : (
                      <span className="detail-value">Unavailable</span>
                    )}
                  </div>
                </div>

                {!walletAccount ? (
                  <div className="wallet-panel-body" style={{ padding: 0 }}>
                    <div className="wallet-section">
                      <label className="wallet-label">Browser Wallet</label>
                      {eip6963Providers.length > 0 ? (
                        <div className="wallet-quick-accounts">
                          {eip6963Providers.map((p, i) => (
                            <button
                              key={p.info?.uuid || i}
                              className="wallet-action-btn wallet-action-primary"
                              onClick={() => connectEip6963(p)}
                              style={{ marginBottom: '8px', width: '100%', display: 'flex', alignItems: 'center', gap: '8px', justifyContent: 'center' }}
                            >
                              {p.info?.icon && <img src={p.info.icon} alt="" style={{ width: 20, height: 20 }} />}
                              {p.info?.name || 'Unknown Wallet'}
                            </button>
                          ))}
                        </div>
                      ) : (
                        <button className="wallet-action-btn wallet-action-primary" onClick={connectMetaMask} style={{ marginBottom: '12px', width: '100%' }}>
                          🦊 Connect MetaMask
                        </button>
                      )}
                    </div>

                    <div className="wallet-section">
                      <label className="wallet-label">WalletConnect</label>
                      <button className="wallet-action-btn wallet-action-primary" onClick={connectWalletConnect} style={{ marginBottom: '12px', width: '100%' }}>
                        📱 Connect via WalletConnect
                      </button>
                    </div>

                    <div className="wallet-section">
                      <label className="wallet-label">Private Key {IS_TESTNET ? '(Testnet Only)' : ''}</label>
                      {!IS_TESTNET && (
                        <div className="wallet-result warning">
                          Direct private key input is disabled on mainnet. Use MetaMask or a hardware wallet.
                        </div>
                      )}
                      {IS_TESTNET && (
                        <>
                          <input
                            type="password"
                            className="wallet-input"
                            placeholder="0x..."
                            autoComplete="off"
                            spellCheck="false"
                            value={walletKeyInput}
                            onChange={(e) => setWalletKeyInput(e.target.value)}
                            onKeyDown={(e) => e.key === 'Enter' && walletKeyInput && connectWallet(walletKeyInput)}
                          />
                          <button className="wallet-action-btn wallet-action-primary" onClick={() => connectWallet(walletKeyInput)} disabled={!walletKeyInput}>
                            Connect
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                ) : (
                  <div className="wallet-panel-body" style={{ padding: 0 }}>
                    <div className="wallet-section">
                      <label className="wallet-label">Connected Address</label>
                      <div className="wallet-address">
                        <CopyButton text={walletAccount.address} label="address" />
                      </div>
                      <div className="wallet-balance-display">
                        <span className="wallet-balance-value">{walletBalance || '...'}</span>
                        <span className="wallet-balance-label">tCREG</span>
                      </div>
                      <div style={{ color: 'var(--text-secondary)', fontSize: '0.85rem', marginTop: '8px' }}>
                        Provider: {walletAccount.providerName || walletAccount.type}
                      </div>
                    </div>

                    <div className="wallet-section">
                      <label className="wallet-label">🚰 Faucet</label>
                      <button className="wallet-action-btn wallet-action-primary" onClick={requestDrip} disabled={dripLoading}>
                        {dripLoading ? 'Requesting...' : 'Request 1,000 tCREG'}
                      </button>
                      {dripResult && (
                        <div className={`wallet-result ${dripResult.success ? 'success' : 'error'}`}>
                          {dripResult.message}
                          {dripResult.tx_hash && <div className="wallet-tx">Tx: {truncateHash(dripResult.tx_hash, 10, 6)}</div>}
                        </div>
                      )}
                    </div>

                    <div className="wallet-section">
                      <label className="wallet-label">⚡ Stake Tokens</label>
                      <input
                        type="number"
                        className="wallet-input"
                        placeholder="Amount (e.g. 100)"
                        value={stakeAmount}
                        onChange={(e) => setStakeAmount(e.target.value)}
                      />
                      <div className="wallet-stake-buttons">
                        <button className="wallet-action-btn wallet-action-primary" onClick={() => doStake('publisher')} disabled={stakeLoading || !stakeAmount}>
                          {stakeLoading ? 'Staking...' : 'Stake as Publisher'}
                        </button>
                        <button className="wallet-action-btn wallet-action-secondary" onClick={() => doStake('validator')} disabled={stakeLoading || !stakeAmount}>
                          {stakeLoading ? 'Staking...' : 'Apply as Validator'}
                        </button>
                      </div>
                      {(!tokenContractAddress || !stakingContractAddress) && (
                        <div className="wallet-result warning">
                          Live staking contracts are not yet available from the running node.
                        </div>
                      )}
                      {stakeResult && (
                        <div className={`wallet-result ${stakeResult.success ? 'success' : 'error'}`}>
                          {stakeResult.message}
                          {stakeResult.tx && <div className="wallet-tx">Tx: {truncateHash(stakeResult.tx, 10, 6)}</div>}
                        </div>
                      )}
                    </div>

                    <div className="wallet-section">
                      <button className="wallet-action-btn wallet-action-danger" onClick={disconnectWallet}>
                        Disconnect
                      </button>
                    </div>
                  </div>
                )}
              </div>
            )}

            {/* Publisher Profile View */}
            {view === 'publisher' && publisherProfile && (
              <div style={{ padding: 'var(--space-4)' }}>
                <button className="nav-tab" onClick={() => setView('packages')} style={{ marginBottom: 'var(--space-3)' }}>
                  ← Back to Packages
                </button>
                <div className="detail-panel" style={{ marginBottom: 'var(--space-4)' }}>
                  <div className="detail-header">
                    <span className="detail-title">👤 Publisher Profile</span>
                  </div>
                  <div className="detail-content">
                    <div className="detail-section">
                      <div className="detail-row">
                        <span className="detail-label">Public Key</span>
                        <CopyButton text={publisherProfile.pubkey} label="pubkey" />
                      </div>
                      <div className="detail-row">
                        <span className="detail-label">Total Packages</span>
                        <span className="detail-value">{publisherProfile.total_packages}</span>
                      </div>
                      <div className="detail-row">
                        <span className="detail-label">Verified</span>
                        <span className="badge badge-success">{publisherProfile.verified_count}</span>
                      </div>
                      <div className="detail-row">
                        <span className="detail-label">Revoked</span>
                        <span className={`badge ${publisherProfile.revoked_count > 0 ? 'badge-error' : 'badge-neutral'}`}>
                          {publisherProfile.revoked_count}
                        </span>
                      </div>
                      <div className="detail-row">
                        <span className="detail-label">Stake</span>
                        <span className="detail-value">{formatStake(publisherProfile.stake_wei || 0)}</span>
                      </div>
                      <div className="detail-row">
                        <span className="detail-label">First Seen</span>
                        <span className="detail-value">
                          {publisherProfile.first_seen_at ? timeAgo(publisherProfile.first_seen_at) : 'N/A'}
                          {publisherProfile.first_seen_days > 0 && ` (${publisherProfile.first_seen_days} days)`}
                        </span>
                      </div>
                    </div>
                  </div>
                </div>

                <div className="detail-section">
                  <div className="detail-section-title">Packages by this Publisher ({publisherPackages.length})</div>
                  {publisherPackages.length === 0 ? (
                    <EmptyState icon="📦" title="No packages found" description="This publisher has no on-chain packages matching the current list." />
                  ) : (
                    <div className="table-container">
                      <table className="data-table">
                        <thead>
                          <tr><th>Package</th><th>Version</th><th>Status</th><th>Published</th></tr>
                        </thead>
                        <tbody>
                          {publisherPackages.map((pkg, idx) => (
                            <tr key={idx}>
                              <td style={{ fontWeight: 600 }}>{pkg.ecosystem}:{pkg.name}</td>
                              <td>{pkg.version}</td>
                              <td><StatusBadge status={pkg.status} /></td>
                              <td>{timeAgo(pkg.published_at)}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  )}
                </div>
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
          {selectedValidator ? (
            <div className="detail-panel">
              <div className="detail-header">
                <span className="detail-title">Validator Details</span>
                <button className="detail-close" onClick={() => setSelectedValidator(null)}>✕</button>
              </div>

              <div className="detail-content">
                <div className="detail-section">
                  <div className="detail-section-title">Identity</div>
                  <div className="detail-row">
                    <span className="detail-label">Validator ID</span>
                    <span className="detail-value">{selectedValidator.id}</span>
                  </div>
                  {selectedValidator.alias && (
                    <div className="detail-row">
                      <span className="detail-label">Alias</span>
                      <span className="detail-value">{selectedValidator.alias}</span>
                    </div>
                  )}
                  <div className="detail-row">
                    <span className="detail-label">Status</span>
                    <StatusBadge status={selectedValidator.status} />
                  </div>
                </div>

                <div className="detail-section">
                  <div className="detail-section-title">Performance</div>
                  <div className="detail-row">
                    <span className="detail-label">Stake</span>
                    <span className="detail-value">{formatStake(selectedValidator.stake || 0)}</span>
                  </div>
                  <div className="detail-row">
                    <span className="detail-label">Reputation</span>
                    <span className="detail-value">{selectedValidator.reputation || 0}/100</span>
                  </div>
                </div>

                <div className="detail-section">
                  <div className="detail-section-title">Operator Actions</div>
                  <div style={{ color: 'var(--text-secondary)', lineHeight: 1.7 }}>
                    Connect a wallet in the Wallet tab to apply as a validator or add stake.<br />
                    Use the Packages tab to inspect pending work and publish package metadata.<br />
                    Use the Network tab to inspect peer connectivity and bridge sync.
                  </div>
                </div>
              </div>
            </div>
          ) : selectedBlock ? (
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
                              {typeof ev.payload === 'object' ? JSON.stringify(ev.payload, null, 2) : ev.payload}
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

      {/* Bridge HUD - Inline */}
      <div className="bridge-hud-inline">
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
              width: bridgeStatus.bridge_sync_status === 'Synced' ? '100%'
                   : bridgeStatus.bridge_sync_progress ? `${Math.min(100, bridgeStatus.bridge_sync_progress)}%`
                   : '0%',
              opacity: bridgeStatus.bridge_sync_status === 'Synced' ? 1 : 0.6
            }} 
          />
        </div>
      </div>
    </div>
  )
}

function AppWithBoundary() {
  return (
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  )
}

export default AppWithBoundary
