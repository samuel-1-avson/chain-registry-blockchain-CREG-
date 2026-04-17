// Chain Registry Explorer - Public Surface
// Features: Blocks, validators, packages, wallet, staking, publish, real-time updates

import React, { useState, useEffect, useRef, useCallback, useMemo } from 'react'
import { createPublicClient, createWalletClient, custom, http, isAddress, parseUnits, formatUnits } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'

// ============================================
// CONFIGURATION
// ============================================

const API_BASE = import.meta.env.VITE_API_BASE || ''
const BUILD_CREG_TOKEN_ADDR = import.meta.env.VITE_CREG_TOKEN || null
const BUILD_STAKING_ADDR = import.meta.env.VITE_STAKING_ADDR || null
const ZERO_ADDRESS = '0x0000000000000000000000000000000000000000'
const DEFAULT_VALIDATOR_REGISTRATION_MODE = 'staking-plus-identity-sync'
const DEFAULT_VALIDATOR_REGISTRATION_NOTE = 'Stake on-chain, register your validator EVM address, node ID, and Ed25519 pubkey with /v1/validators/register, wait for governance approval, and the node sync loop will admit active validators into consensus automatically.'
// Private key input is only enabled in Vite dev mode (import.meta.env.DEV).
// VITE_DEV_MODE is intentionally NOT checked here — env vars are baked into the
// production bundle and a misconfigured server could expose this to end users.
const PRIVATE_KEY_WALLET_ENABLED = import.meta.env.DEV === true
const DEFAULT_NETWORK_PROFILE_ID = import.meta.env.VITE_DEFAULT_NETWORK_PROFILE || 'anvil'

const buildChainConfig = (id, name, rpcUrl, nativeCurrency = { name: 'Ether', symbol: 'ETH', decimals: 18 }) => ({
  id,
  name,
  nativeCurrency,
  rpcUrls: { default: { http: [rpcUrl] } },
})

const PUBLIC_TESTNET_PROFILES = {
  sepolia: {
    id: 'sepolia',
    label: 'Ethereum Sepolia',
    shortLabel: 'Sepolia',
    description: 'Public Ethereum app and contract testnet.',
    purpose: 'Application and contract testing',
    chainId: 11155111,
    rpcUrl: import.meta.env.VITE_SEPOLIA_RPC_URL || 'https://ethereum-sepolia-rpc.publicnode.com',
    faucetUrl: import.meta.env.VITE_SEPOLIA_FAUCET_URL || 'https://sepolia-faucet.pk910.de/',
    blockExplorerUrl: import.meta.env.VITE_SEPOLIA_BLOCK_EXPLORER_URL || 'https://sepolia.etherscan.io',
    tokenContract: import.meta.env.VITE_SEPOLIA_CREG_TOKEN || null,
    stakingContract: import.meta.env.VITE_SEPOLIA_STAKING_ADDR || null,
    registryAddress: import.meta.env.VITE_SEPOLIA_REGISTRY_ADDR || null,
    validatorRegistrationMode: 'public-testnet',
    validatorRegistrationNote: 'Sepolia is the recommended public Ethereum testnet for app and contract testing. Configure VITE_SEPOLIA_* contract addresses before using staking or publishing from this explorer on Sepolia.',
    directFunding: false,
  },
  hoodi: {
    id: 'hoodi',
    label: 'Ethereum Hoodi',
    shortLabel: 'Hoodi',
    description: 'Public Ethereum validator and staking testnet.',
    purpose: 'Validator, staking, and protocol-upgrade testing',
    chainId: 560048,
    rpcUrl: import.meta.env.VITE_HOODI_RPC_URL || 'https://rpc.hoodi.ethpandaops.io',
    faucetUrl: import.meta.env.VITE_HOODI_FAUCET_URL || 'https://hoodi-faucet.pk910.de/',
    blockExplorerUrl: import.meta.env.VITE_HOODI_BLOCK_EXPLORER_URL || 'https://hoodi.etherscan.io',
    tokenContract: import.meta.env.VITE_HOODI_CREG_TOKEN || null,
    stakingContract: import.meta.env.VITE_HOODI_STAKING_ADDR || null,
    registryAddress: import.meta.env.VITE_HOODI_REGISTRY_ADDR || null,
    validatorRegistrationMode: 'public-validator-testnet',
    validatorRegistrationNote: 'Hoodi is the public Ethereum testnet for validator and staking flows. Configure VITE_HOODI_* contract addresses before using direct on-chain staking from this explorer on Hoodi.',
    directFunding: false,
  },
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

const PERMIT_TYPED_DATA_TYPES = {
  Permit: [
    { name: 'owner', type: 'address' },
    { name: 'spender', type: 'address' },
    { name: 'value', type: 'uint256' },
    { name: 'nonce', type: 'uint256' },
    { name: 'deadline', type: 'uint256' },
  ],
}

const SPONSORED_STAKE_INTENT_TYPES = {
  SponsoredStakeIntent: [
    { name: 'owner', type: 'address' },
    { name: 'tokenContract', type: 'address' },
    { name: 'stakingContract', type: 'address' },
    { name: 'action', type: 'uint8' },
    { name: 'amount', type: 'uint256' },
    { name: 'permitNonce', type: 'uint256' },
    { name: 'permitDeadline', type: 'uint256' },
    { name: 'relayerNonce', type: 'uint256' },
    { name: 'expiresAt', type: 'uint256' },
  ],
}

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

/**
 * Mine a proof-of-work nonce: find a value such that
 * SHA-256(challenge + nonce) has `difficulty` leading zero bits.
 * Uses the Web Crypto API for fast hashing.
 */
const minePoW = async (challenge, difficulty) => {
  const encoder = new TextEncoder()
  for (let nonce = 0; nonce < 0x7FFFFFFF; nonce++) {
    const nonceStr = nonce.toString()
    const data = encoder.encode(challenge + nonceStr)
    const hashBuf = await crypto.subtle.digest('SHA-256', data)
    const hash = new Uint8Array(hashBuf)
    let leadingZeros = 0
    for (const byte of hash) {
      if (byte === 0) { leadingZeros += 8 }
      else { leadingZeros += Math.clz32(byte) - 24; break }
      if (leadingZeros >= difficulty) break
    }
    if (leadingZeros >= difficulty) return nonceStr
  }
  throw new Error('PoW mining exhausted — could not find a valid nonce.')
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

const CopyTextButton = ({ text, label, children = 'Copy', className = '', title }) => {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch (err) {
      console.error(`Failed to copy ${label}:`, err)
    }
  }

  return (
    <button
      type="button"
      className={`wallet-inline-action ${className}`.trim()}
      onClick={handleCopy}
      title={title || `Copy ${label}`}
    >
      {copied ? 'Copied' : children}
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
  const [walletNativeBalance, setWalletNativeBalance] = useState(null)
  const [walletRpcOffline, setWalletRpcOffline] = useState(false)
  const walletBalanceFailuresRef = useRef(0)
  const [walletFundingLoading, setWalletFundingLoading] = useState(false)
  const [walletFundingResult, setWalletFundingResult] = useState(null)
  const [walletFundingCooldownSecs, setWalletFundingCooldownSecs] = useState(0)
  const [relayerPolicy, setRelayerPolicy] = useState(null)
  const [relayerPolicyError, setRelayerPolicyError] = useState(null)
  const [activeNetworkProfileId, setActiveNetworkProfileId] = useState(() => {
    if (typeof window === 'undefined') return DEFAULT_NETWORK_PROFILE_ID
    return window.localStorage.getItem('creg.walletNetworkProfile') || DEFAULT_NETWORK_PROFILE_ID
  })
  const [walletKeyInput, setWalletKeyInput] = useState('')
  const [stakeLoading, setStakeLoading] = useState(false)
  const [sponsoredStakeLoading, setSponsoredStakeLoading] = useState(false)
  const [stakeResult, setStakeResult] = useState(null)
  const [stakeTxHistory, setStakeTxHistory] = useState(() => {
    try {
      const raw = window.localStorage.getItem('creg.stakeTxHistory')
      return raw ? JSON.parse(raw) : []
    } catch { return [] }
  })
  const [stakeAmount, setStakeAmount] = useState('')
  const [validatorRegistrations, setValidatorRegistrations] = useState([])
  const [validatorIdentityForm, setValidatorIdentityForm] = useState({ alias: '', nodeId: '', ed25519Pubkey: '' })
  const [validatorRegistrationLoading, setValidatorRegistrationLoading] = useState(false)
  const [validatorRegistrationResult, setValidatorRegistrationResult] = useState(null)
  const [runtimeConfig, setRuntimeConfig] = useState({
    tokenContract: normalizeContractAddress(BUILD_CREG_TOKEN_ADDR),
    stakingContract: normalizeContractAddress(BUILD_STAKING_ADDR),
    registryAddress: null,
    isTestnet: IS_TESTNET,
    validatorRegistrationMode: DEFAULT_VALIDATOR_REGISTRATION_MODE,
    validatorRegistrationNote: DEFAULT_VALIDATOR_REGISTRATION_NOTE,
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
  const [packageFilterText, setPackageFilterText] = useState('')
  const [showPublishForm, setShowPublishForm] = useState(false)
  const [publishForm, setPublishForm] = useState({ ecosystem: 'npm', name: '', version: '', ipfs_cid: '', content_hash: '', publisher_pubkey: '', signature: '' })
  const [publishStatus, setPublishStatus] = useState(null)
  const [publishErrors, setPublishErrors] = useState({})
  const [publisherProfile, setPublisherProfile] = useState(null)
  const [publisherPackages, setPublisherPackages] = useState([])

  // SSE connection health (WEB-H02)
  const [sseConnected, setSseConnected] = useState(false)
  const [sseReconnectIn, setSseReconnectIn] = useState(0)
  const sseReconnectTimerRef = useRef(null)

  const browserHost = typeof window !== 'undefined' && window.location.hostname ? window.location.hostname : '127.0.0.1'
  const explorerOrigin = typeof window !== 'undefined' ? window.location.origin : 'http://127.0.0.1:3000'
  const directNodeUrl = `http://${browserHost}:8080`
  const directFaucetUrl = `http://${browserHost}:8082`
  const directRpcUrl = `http://${browserHost}:8545`
  const directRelayerUrl = import.meta.env.VITE_RELAYER_URL || `http://${browserHost}:8083`

  const sseRef = useRef(null)
  const searchInputRef = useRef(null)
  const pollTimerRef = useRef(null)
  const pollDelayRef = useRef(5000)
  const blocksRef = useRef([])

  const networkProfiles = useMemo(() => ({
    anvil: {
      id: 'anvil',
      label: 'Local Anvil',
      shortLabel: 'Anvil',
      description: 'Fast local development chain with direct in-app faucet funding.',
      purpose: 'Fast local iteration',
      chain: buildChainConfig(31337, 'Anvil Local', directRpcUrl),
      rpcUrl: directRpcUrl,
      faucetUrl: directFaucetUrl,
      faucetApiBase: `${API_BASE}/api`,
      faucetApiUrl: `${API_BASE}/api/drip`,
      blockExplorerUrl: explorerOrigin,
      tokenContract: BUILD_CREG_TOKEN_ADDR,
      stakingContract: BUILD_STAKING_ADDR,
      registryAddress: runtimeConfig.registryAddress,
      validatorRegistrationMode: runtimeConfig.validatorRegistrationMode,
      validatorRegistrationNote: runtimeConfig.validatorRegistrationNote,
      directFunding: true,
    },
    sepolia: {
      ...PUBLIC_TESTNET_PROFILES.sepolia,
      chain: buildChainConfig(PUBLIC_TESTNET_PROFILES.sepolia.chainId, PUBLIC_TESTNET_PROFILES.sepolia.label, PUBLIC_TESTNET_PROFILES.sepolia.rpcUrl),
      faucetApiUrl: null,
    },
    hoodi: {
      ...PUBLIC_TESTNET_PROFILES.hoodi,
      chain: buildChainConfig(PUBLIC_TESTNET_PROFILES.hoodi.chainId, PUBLIC_TESTNET_PROFILES.hoodi.label, PUBLIC_TESTNET_PROFILES.hoodi.rpcUrl, { name: 'Hoodi Ether', symbol: 'ETH', decimals: 18 }),
      faucetApiUrl: null,
    },
  }), [directRpcUrl, directFaucetUrl, explorerOrigin, runtimeConfig.registryAddress, runtimeConfig.validatorRegistrationMode, runtimeConfig.validatorRegistrationNote])

  useEffect(() => {
    if (networkProfiles[activeNetworkProfileId]) {
      if (typeof window !== 'undefined') {
        window.localStorage.setItem('creg.walletNetworkProfile', activeNetworkProfileId)
      }
      return
    }
    setActiveNetworkProfileId(DEFAULT_NETWORK_PROFILE_ID in networkProfiles ? DEFAULT_NETWORK_PROFILE_ID : 'anvil')
  }, [activeNetworkProfileId, networkProfiles])

  const activeNetworkProfile = useMemo(
    () => networkProfiles[activeNetworkProfileId] || networkProfiles[DEFAULT_NETWORK_PROFILE_ID] || networkProfiles.anvil,
    [activeNetworkProfileId, networkProfiles]
  )

  const activeChain = activeNetworkProfile.chain
  const activeRpcUrl = activeNetworkProfile.rpcUrl
  const activeFaucetUrl = activeNetworkProfile.faucetUrl

  const tokenContractAddress = useMemo(
    () => activeNetworkProfile.id === 'anvil'
      ? (normalizeContractAddress(runtimeConfig.tokenContract) || normalizeContractAddress(BUILD_CREG_TOKEN_ADDR))
      : normalizeContractAddress(activeNetworkProfile.tokenContract),
    [activeNetworkProfile, runtimeConfig.tokenContract]
  )

  const stakingContractAddress = useMemo(
    () => activeNetworkProfile.id === 'anvil'
      ? (normalizeContractAddress(runtimeConfig.stakingContract) || normalizeContractAddress(BUILD_STAKING_ADDR))
      : normalizeContractAddress(activeNetworkProfile.stakingContract),
    [activeNetworkProfile, runtimeConfig.stakingContract]
  )

  const activeRegistryAddress = useMemo(
    () => activeNetworkProfile.id === 'anvil'
      ? normalizeContractAddress(runtimeConfig.registryAddress)
      : normalizeContractAddress(activeNetworkProfile.registryAddress),
    [activeNetworkProfile, runtimeConfig.registryAddress]
  )

  const activeValidatorRegistrationMode = activeNetworkProfile.id === 'anvil'
    ? runtimeConfig.validatorRegistrationMode
    : activeNetworkProfile.validatorRegistrationMode

  const activeValidatorRegistrationNote = activeNetworkProfile.id === 'anvil'
    ? runtimeConfig.validatorRegistrationNote
    : activeNetworkProfile.validatorRegistrationNote

  const activeProfileHasContracts = Boolean(tokenContractAddress && stakingContractAddress)
  const activeFundingActionLabel = activeNetworkProfile.directFunding ? '💧 Fund Wallet' : `💧 Open ${activeNetworkProfile.shortLabel} Faucet`
  const activeFundingHelp = activeNetworkProfile.directFunding
    ? 'The local faucet can fund the connected address directly from the explorer.'
    : `${activeNetworkProfile.label} uses external public faucets. The explorer can switch your wallet network cleanly, but public faucets still require opening the faucet site for manual completion.`
  const walletFundingCooldownActive = activeNetworkProfile.directFunding && walletFundingCooldownSecs > 0
  const walletFundingButtonLabel = walletFundingLoading
    ? 'Funding...'
    : walletFundingCooldownActive
      ? `Retry in ${walletFundingCooldownSecs}s`
      : activeFundingActionLabel

  const activeRelayerChainPolicy = useMemo(
    () => relayerPolicy?.chains?.find((chain) => Number(chain.id) === activeChain.id && chain.enabled) || null,
    [relayerPolicy, activeChain.id]
  )

  const activeSponsoredPublisherPolicy = useMemo(
    () => activeRelayerChainPolicy?.actions?.find((action) => action.key === 'publisher') || null,
    [activeRelayerChainPolicy]
  )

  const activeSponsoredValidatorPolicy = useMemo(
    () => activeRelayerChainPolicy?.actions?.find((action) => action.key === 'validator') || null,
    [activeRelayerChainPolicy]
  )

  const activeRelayerHelp = activeRelayerChainPolicy
    ? `Sponsored actions are available on ${activeNetworkProfile.label}. Your wallet signs permit and relayer-intent payloads, and the relayer pays gas for the staking transaction.`
    : relayerPolicyError
      ? `Sponsored actions are unavailable: ${relayerPolicyError}`
      : `Sponsored actions are not configured for ${activeNetworkProfile.label} on the currently reachable relayer.`

  const walletValidatorRegistration = useMemo(() => {
    if (!walletAccount?.address) return null
    const walletAddress = walletAccount.address.toLowerCase()
    return validatorRegistrations.find((registration) => registration?.identity?.evm_address?.toLowerCase() === walletAddress) || null
  }, [walletAccount, validatorRegistrations])

  const validatorLifecycle = useMemo(() => {
    const registration = walletValidatorRegistration
    return [
      { key: 'registered', label: 'Identity registered with node', complete: Boolean(registration?.registered_with_node) },
      { key: 'applied', label: 'Applied on-chain', complete: Boolean(registration?.applied_on_chain) },
      { key: 'approved', label: 'Governance approved', complete: Boolean(registration?.governance_approved) },
      { key: 'admitted', label: 'Admitted to consensus', complete: Boolean(registration?.admitted_to_consensus) },
      { key: 'active', label: 'Active', complete: Boolean(registration?.active) },
    ]
  }, [walletValidatorRegistration])

  const detectedValidatorNodes = useMemo(
    () => nodes.filter((node) => node?.id && node?.pubkey),
    [nodes]
  )

  const validatorStatusCommand = useMemo(
    () => `Invoke-RestMethod -Uri "${directNodeUrl}/v1/validators/registrations" -TimeoutSec 10 | ConvertTo-Json -Depth 8`,
    [directNodeUrl]
  )

  const validatorRegistrationCommand = useMemo(() => {
    const payload = {
      evm_address: walletAccount?.address || '0x<validator-wallet>',
      node_id: validatorIdentityForm.nodeId.trim() || 'node-2',
      ed25519_pubkey: validatorIdentityForm.ed25519Pubkey.trim() || '<validator-ed25519-pubkey>',
      alias: validatorIdentityForm.alias.trim() || validatorIdentityForm.nodeId.trim() || 'Validator-2',
    }
    const jsonPayload = JSON.stringify(payload).replace(/'/g, "''")
    return `$body = '${jsonPayload}'; Invoke-RestMethod -Method Post -Uri "${directNodeUrl}/v1/validators/register" -ContentType "application/json" -Body $body`
  }, [walletAccount?.address, validatorIdentityForm, directNodeUrl])

  const adoptNodeIdentity = useCallback((node) => {
    setValidatorIdentityForm((current) => ({
      alias: current.alias || node.alias || node.id,
      nodeId: node.id,
      ed25519Pubkey: node.pubkey,
    }))
    setValidatorRegistrationResult(null)
  }, [])

  useEffect(() => {
    blocksRef.current = blocks
  }, [blocks])

  const fetchBlockByHeight = useCallback(async (height) => {
    const response = await fetch(`${API_BASE}/v1/blocks/${height}`)
    if (response.status === 429) {
      throw new Error('429 rate limit exceeded')
    }
    if (!response.ok) return null
    return await response.json()
  }, [])

  // Record a successful staking tx into localStorage history (WEB-M02).
  const recordStakeTx = useCallback((type, amount, txHash) => {
    const entry = {
      type,           // e.g. 'publisher' | 'validator' | 'sponsored'
      amount,
      txHash: txHash || null,
      network: activeNetworkProfileId,
      at: new Date().toISOString(),
    }
    setStakeTxHistory(prev => {
      const next = [entry, ...prev].slice(0, 50) // keep last 50
      try {
        window.localStorage.setItem('creg.stakeTxHistory', JSON.stringify(next))
      } catch { /* storage full — ignore */ }
      return next
    })
  }, [activeNetworkProfileId])

  const refreshRecentBlocks = useCallback(async (tipHeight) => {
    const currentBlocks = blocksRef.current
    const currentTopHeight = currentBlocks[0]?.header?.height

    if (view !== 'blocks' && currentBlocks.length > 0) {
      return
    }

    if (currentBlocks.length === 0) {
      const initialLimit = 12
      const startHeight = Math.max(0, tipHeight - (initialLimit - 1))
      const heights = []
      for (let height = tipHeight; height >= startHeight; height -= 1) {
        heights.push(height)
      }
      const fetchedBlocks = (await Promise.all(heights.map(fetchBlockByHeight))).filter(Boolean)
      if (fetchedBlocks.length > 0) {
        setBlocks(fetchedBlocks)
      }
      return
    }

    if (typeof currentTopHeight !== 'number' || tipHeight <= currentTopHeight) {
      return
    }

    const newHeights = []
    for (let height = tipHeight; height > currentTopHeight && newHeights.length < 6; height -= 1) {
      newHeights.push(height)
    }

    const fetchedBlocks = (await Promise.all(newHeights.map(fetchBlockByHeight))).filter(Boolean)
    if (fetchedBlocks.length === 0) {
      return
    }

    setBlocks((current) => {
      const merged = [...fetchedBlocks, ...current]
      const deduped = []
      const seenHeights = new Set()
      for (const block of merged) {
        const height = block?.header?.height
        if (typeof height !== 'number' || seenHeights.has(height)) continue
        seenHeights.add(height)
        deduped.push(block)
        if (deduped.length >= 20) break
      }
      return deduped
    })
  }, [fetchBlockByHeight, view])

  // Fetch data
  const fetchData = useCallback(async () => {
    try {
      const [statsRes, nodesRes, p2pRes, bridgeRes, runtimeRes, validatorRegistrationsRes] = await Promise.all([
        fetch(`${API_BASE}/v1/chain/stats`),
        fetch(`${API_BASE}/v1/nodes`),
        fetch(`${API_BASE}/v1/p2p/status`),
        fetch(`${API_BASE}/v1/bridge/status`),
        fetch(`${API_BASE}/v1/runtime/config`).catch(() => null),
        fetch(`${API_BASE}/v1/validators/registrations`).catch(() => null)
      ])

      const primaryResponses = [statsRes, nodesRes, p2pRes, bridgeRes, runtimeRes, validatorRegistrationsRes].filter(Boolean)
      if (primaryResponses.some((response) => response.status === 429)) {
        throw new Error('429 rate limit exceeded')
      }
      
      if (statsRes.ok) {
        const statsData = await statsRes.json()
        setStats(statsData)
        await refreshRecentBlocks(statsData.tip_height)
      }

      if (nodesRes.ok) setNodes(await nodesRes.json())
      if (p2pRes.ok) setP2pStatus(await p2pRes.json())
      if (bridgeRes.ok) setBridgeStatus(await bridgeRes.json())
      if (validatorRegistrationsRes?.ok) setValidatorRegistrations(await validatorRegistrationsRes.json())
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
              validatorRegistrationMode: runtimeData.validator_registration_mode || DEFAULT_VALIDATOR_REGISTRATION_MODE,
              validatorRegistrationNote: runtimeData.validator_registration_note || DEFAULT_VALIDATOR_REGISTRATION_NOTE,
            })
          } catch (runtimeError) {
            console.warn('Ignoring invalid runtime config payload:', runtimeError)
          }
        } else {
          console.warn('Ignoring non-JSON runtime config response')
        }
      }

      // Fetch pending packages (mempool)
      try {
        const pendingRes = await fetch(`${API_BASE}/v1/pending`)
        if (pendingRes.status === 429) {
          throw new Error('429 rate limit exceeded')
        }
        if (pendingRes.ok) setPendingPackages(await pendingRes.json())
      } catch (e) {
        if (`${e?.message || ''}`.includes('429')) {
          throw e
        }
        /* endpoint may not exist yet */
      }

      // Fetch finalized/verified packages so they stay visible after leaving
      // the pending pool (packages disappear from /v1/pending once committed).
      try {
        const pkgRes = await fetch(`${API_BASE}/v1/packages?offset=0&limit=20`)
        if (pkgRes.ok) {
          const pkgData = await pkgRes.json()
          setPackageList(pkgData)
        }
      } catch (e) { /* non-fatal */ }

      setStatus('online')
      setFetchError(null)
      setIsLoading(false)
      pollDelayRef.current = 10000
    } catch (err) {
      console.error('Fetch error:', err)
      const isRateLimited = `${err?.message || ''}`.includes('429')
      setFetchError(isRateLimited ? 'Node API is rate limiting the explorer. Backing off and retrying.' : (err.message || 'Failed to connect to node'))
      setStatus(isRateLimited ? 'syncing' : 'offline')
      setIsLoading(false)
      if (isRateLimited) {
        pollDelayRef.current = Math.min(pollDelayRef.current * 2, 30000)
      }
    }
  }, [refreshRecentBlocks])

  // Initial fetch and polling
  useEffect(() => {
    let cancelled = false

    const scheduleNextPoll = () => {
      if (cancelled) return
      pollTimerRef.current = setTimeout(async () => {
        await fetchData()
        scheduleNextPoll()
      }, pollDelayRef.current)
    }

    fetchData().finally(scheduleNextPoll)

    return () => {
      cancelled = true
      if (pollTimerRef.current) clearTimeout(pollTimerRef.current)
    }
  }, [fetchData])

  // SSE Event Stream (WEB-H02: reconnect logic + connection banner)
  useEffect(() => {
    let retryCount = 0
    const MAX_RETRIES = 20
    let retryTimeout = null
    let countdownInterval = null

    const startCountdown = (delaySecs) => {
      setSseReconnectIn(delaySecs)
      if (countdownInterval) clearInterval(countdownInterval)
      countdownInterval = setInterval(() => {
        setSseReconnectIn(prev => {
          if (prev <= 1) { clearInterval(countdownInterval); return 0 }
          return prev - 1
        })
      }, 1000)
    }

    const initSSE = () => {
      const es = new EventSource(`${API_BASE}/v1/events`)
      es.onopen = () => {
        retryCount = 0
        setSseConnected(true)
        setSseReconnectIn(0)
        if (countdownInterval) clearInterval(countdownInterval)
      }
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
        setSseConnected(false)
        if (retryCount < MAX_RETRIES) {
          const delaySecs = Math.min(Math.pow(2, retryCount), 30)
          retryCount++
          startCountdown(delaySecs)
          retryTimeout = setTimeout(initSSE, delaySecs * 1000)
        }
      }
      sseRef.current = es
    }

    initSSE()
    return () => {
      sseRef.current?.close()
      if (retryTimeout) clearTimeout(retryTimeout)
      if (countdownInterval) clearInterval(countdownInterval)
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
      const publicClient = createPublicClient({ chain: activeChain, transport: http(activeRpcUrl) })
      const nativeBalance = await publicClient.getBalance({ address: walletAccount.address })
      setWalletNativeBalance(formatUnits(nativeBalance, 18))

      if (tokenContractAddress) {
        const rawBalance = await publicClient.readContract({
          address: tokenContractAddress,
          abi: ERC20_ABI,
          functionName: 'balanceOf',
          args: [walletAccount.address],
        })
        setWalletBalance(formatUnits(rawBalance, 18))
      } else {
        setWalletBalance('0')
      }
      walletBalanceFailuresRef.current = 0
      setWalletRpcOffline(false)
    } catch (e) {
      walletBalanceFailuresRef.current += 1
      if (walletBalanceFailuresRef.current === 1) {
        console.warn(`Wallet balance refresh failed (RPC ${activeRpcUrl}): ${e.shortMessage || e.message || e}`)
      }
      if (walletBalanceFailuresRef.current >= 3) setWalletRpcOffline(true)
    }
  }, [walletAccount, tokenContractAddress, activeChain, activeRpcUrl])

  useEffect(() => {
    walletBalanceFailuresRef.current = 0
    setWalletRpcOffline(false)
  }, [activeRpcUrl, walletAccount?.address])

  useEffect(() => {
    if (!walletAccount) return undefined
    let cancelled = false
    let timer = null
    const tick = async () => {
      if (cancelled) return
      await refreshWalletBalance()
      if (cancelled) return
      const delay = walletBalanceFailuresRef.current >= 3 ? 60000 : 10000
      timer = setTimeout(tick, delay)
    }
    tick()
    return () => {
      cancelled = true
      if (timer) clearTimeout(timer)
    }
  }, [walletAccount, refreshWalletBalance])

  useEffect(() => {
    if (walletFundingCooldownSecs <= 0) return undefined

    const timer = window.setTimeout(() => {
      setWalletFundingCooldownSecs((current) => Math.max(0, current - 1))
    }, 1000)

    return () => window.clearTimeout(timer)
  }, [walletFundingCooldownSecs])

  useEffect(() => {
    setWalletFundingResult(null)
    setWalletFundingCooldownSecs(0)
  }, [walletAccount?.address, activeNetworkProfileId])

  const refreshRelayerPolicy = useCallback(async () => {
    try {
      const response = await fetch(`${directRelayerUrl}/v1/relayer/policy`)
      const payload = await response.json().catch(() => null)
      if (!response.ok) {
        throw new Error(payload?.error || `Relayer policy request failed with status ${response.status}`)
      }
      setRelayerPolicy(payload)
      setRelayerPolicyError(null)
    } catch (err) {
      setRelayerPolicy(null)
      setRelayerPolicyError(err.message || 'Failed to reach the relayer service.')
    }
  }, [directRelayerUrl])

  useEffect(() => {
    refreshRelayerPolicy()
  }, [refreshRelayerPolicy])

  useEffect(() => {
    if (!walletAccount?.address) {
      setValidatorIdentityForm({ alias: '', nodeId: '', ed25519Pubkey: '' })
      setValidatorRegistrationResult(null)
      return
    }
    setValidatorRegistrationResult(null)
  }, [walletAccount?.address])

  useEffect(() => {
    if (!walletValidatorRegistration) return
    setValidatorIdentityForm((current) => {
      if (current.nodeId || current.ed25519Pubkey || current.alias) return current
      return {
        alias: walletValidatorRegistration.alias || walletValidatorRegistration.identity?.node_id || '',
        nodeId: walletValidatorRegistration.identity?.node_id || '',
        ed25519Pubkey: walletValidatorRegistration.identity?.ed25519_pubkey || '',
      }
    })
  }, [walletValidatorRegistration])

  const ensureWalletChain = useCallback(async (provider, profile = activeNetworkProfile) => {
    if (!provider?.request) return

    const targetChainId = `0x${profile.chain.id.toString(16)}`
    const currentChainId = await provider.request({ method: 'eth_chainId' })
    if (currentChainId === targetChainId) return

    const addParams = {
      chainId: targetChainId,
      chainName: profile.chain.name,
      nativeCurrency: profile.chain.nativeCurrency,
      rpcUrls: [profile.rpcUrl],
      blockExplorerUrls: profile.blockExplorerUrl ? [profile.blockExplorerUrl] : [],
    }

    // Anvil (31337) is typically preconfigured in MetaMask as "Localhost 8545" or similar
    // user-custom. Try switch first; fall back to add only on 4902.
    // Public testnets (Sepolia, Hoodi, etc.) are often NOT in MetaMask — add-first is
    // idempotent (switches if already present) and avoids the noisy 4902 pre-fallback.
    const addFirst = profile.id !== 'anvil'

    const doSwitch = () => provider.request({
      method: 'wallet_switchEthereumChain',
      params: [{ chainId: targetChainId }],
    })
    const doAdd = () => provider.request({
      method: 'wallet_addEthereumChain',
      params: [addParams],
    })

    if (addFirst) {
      try {
        await doAdd()
      } catch (addError) {
        // Some wallets require an explicit switch after add; retry via switch.
        if (addError?.code === 4001) throw addError // user rejected
        await doSwitch()
      }
      return
    }

    try {
      await doSwitch()
    } catch (switchError) {
      if (switchError?.code !== 4902 && !`${switchError?.message || ''}`.includes('Unrecognized chain')) {
        throw switchError
      }
      await doAdd()
    }
  }, [activeNetworkProfile])

  const selectNetworkProfile = useCallback(async (profileId) => {
    const nextProfile = networkProfiles[profileId]
    if (!nextProfile) return

    setActiveNetworkProfileId(profileId)
    setStakeResult(null)
    setWalletFundingResult(null)

    if (walletProvider?.request) {
      try {
        await ensureWalletChain(walletProvider, nextProfile)
      } catch (err) {
        alert(`Failed to switch wallet to ${nextProfile.label}: ${err.message || err}`)
      }
    }
  }, [networkProfiles, walletProvider, ensureWalletChain])

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
    setStakeResult(null)
  }, [ensureWalletChain])

  const connectWallet = useCallback(async (privateKey) => {
    if (!PRIVATE_KEY_WALLET_ENABLED) {
      alert('Direct private key input is only available in local dev mode. Use MetaMask or WalletConnect instead.')
      return
    }
    try {
      const key = privateKey.startsWith('0x') ? privateKey : `0x${privateKey}`
      const account = privateKeyToAccount(key)
      setWalletProvider(null)
      setWalletAccount({ address: account.address, type: 'local', providerName: 'Private Key', account })
      setWalletKeyInput('')
      setStakeResult(null)
      setWalletFundingResult(null)
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
        chains: [activeChain.id],
        rpcMap: { [activeChain.id]: activeRpcUrl },
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
      setStakeResult(null)
    } catch (err) {
      if (err?.message?.includes('User rejected') || err?.code === 4001) {
        // User closed the modal — not an error
        return
      }
      alert('WalletConnect failed: ' + (err.message || err))
    }
  }, [activeChain.id, activeRpcUrl])

  const disconnectWallet = useCallback(() => {
    if (walletProvider?.disconnect) {
      Promise.resolve(walletProvider.disconnect()).catch(() => {})
    }
    setWalletProvider(null)
    setWalletAccount(null)
    setWalletBalance(null)
    setWalletNativeBalance(null)
    setWalletFundingLoading(false)
    setWalletFundingResult(null)
    setWalletFundingCooldownSecs(0)
    setRelayerPolicyError((current) => current)
    setStakeResult(null)
    setStakeAmount('')
    setValidatorRegistrationResult(null)
  }, [walletProvider])

  const createSigningWalletClient = useCallback(() => {
    if (!walletAccount) {
      throw new Error('Connect a wallet before requesting sponsorship.')
    }
    if (walletProvider) {
      return createWalletClient({ chain: activeChain, transport: custom(walletProvider) })
    }
    if (walletAccount.account) {
      return createWalletClient({ account: walletAccount.account, chain: activeChain, transport: http(activeRpcUrl) })
    }
    throw new Error('Wallet signer unavailable for sponsored actions.')
  }, [walletAccount, walletProvider, activeChain, activeRpcUrl])

  const pollSponsoredRequest = useCallback(async (requestId, role, requestedAmount, initialTxHash = null) => {
    for (let attempt = 0; attempt < 30; attempt += 1) {
      const response = await fetch(`${directRelayerUrl}/v1/relayer/status/${requestId}`)
      const payload = await response.json().catch(() => null)
      if (!response.ok || !payload) {
        break
      }

      const txHash = payload.txHash || initialTxHash || null
      if (payload.status === 'confirmed') {
        const confirmedMessage = role === 'publisher'
          ? `Sponsored publisher stake confirmed for ${requestedAmount} tCREG.`
          : `Sponsored validator application confirmed for ${requestedAmount} tCREG.`
        setStakeResult({ success: true, message: payload.message || confirmedMessage, tx: txHash })
        recordStakeTx(`sponsored-${role}`, requestedAmount, txHash)
        await refreshWalletBalance()
        await fetchData()
        return
      }

      if (payload.status === 'failed' || payload.status === 'timed_out' || payload.status === 'rejected') {
        setStakeResult({ success: false, message: payload.message || `Sponsored ${role} action failed.`, tx: txHash })
        return
      }

      await new Promise((resolve) => window.setTimeout(resolve, 2000))
    }
  }, [directRelayerUrl, refreshWalletBalance, fetchData, recordStakeTx])

  const fundConnectedWallet = useCallback(async () => {
    if (!walletAccount?.address) return

    if (walletFundingCooldownActive) {
      setWalletFundingResult({
        success: false,
        message: `Faucet cooldown active. Try again in ${walletFundingCooldownSecs} second${walletFundingCooldownSecs === 1 ? '' : 's'}.`,
      })
      return
    }

    setWalletFundingLoading(true)
    setWalletFundingResult(null)

    try {
      if (!activeNetworkProfile.directFunding || !activeNetworkProfile.faucetApiUrl) {
        if (activeFaucetUrl) {
          window.open(activeFaucetUrl, '_blank', 'noopener,noreferrer')
          setWalletFundingResult({
            success: true,
            message: `${activeNetworkProfile.label} uses an external public faucet. Opened the faucet in a new tab. Public faucets usually require captcha, login, or manual approval, so direct in-app funding is only available on Local Anvil.`,
          })
          return
        }
        throw new Error(`No faucet URL is configured for ${activeNetworkProfile.label}.`)
      }

      // Attempt PoW challenge flow: request a challenge, mine, then drip.
      // If the faucet has PoW disabled it will accept a bare {address} too,
      // but we always try the challenge path first for correctness.
      const faucetBase = activeNetworkProfile.faucetApiBase || activeNetworkProfile.faucetApiUrl.replace(/\/drip$/, '')
      let challenge = null
      let nonce = null
      try {
        const challengeRes = await fetch(`${faucetBase}/challenge`)
        if (challengeRes.ok) {
          const challengeData = await challengeRes.json()
          challenge = challengeData.challenge
          nonce = await minePoW(challenge, challengeData.difficulty)
        }
      } catch (_) {
        // Challenge endpoint unavailable — proceed without PoW
      }

      const dripBody = { address: walletAccount.address }
      if (challenge && nonce) {
        dripBody.challenge = challenge
        dripBody.nonce = nonce
      }

      const response = await fetch(activeNetworkProfile.faucetApiUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(dripBody),
      })
      const payload = await response.json().catch(() => null)
      const retryAfterHeader = Number(response.headers.get('Retry-After') || 0)
      const retryAfterSeconds = Number(payload?.retry_after_seconds || retryAfterHeader || 0)

      if (!response.ok || !payload?.success) {
        if (response.status === 429 && retryAfterSeconds > 0) {
          setWalletFundingCooldownSecs(retryAfterSeconds)
        }
        throw new Error(payload?.message || `Faucet request failed with status ${response.status}`)
      }

      const cooldownSeconds = Number(payload?.cooldown_seconds || 0)
      if (cooldownSeconds > 0) {
        setWalletFundingCooldownSecs(cooldownSeconds)
      }

      setWalletFundingResult({
        success: true,
        message: payload.message || 'Wallet funded successfully.',
        tokenTxHash: payload.token_tx_hash || null,
        nativeTxHash: payload.native_tx_hash || null,
      })
      await refreshWalletBalance()
    } catch (err) {
      setWalletFundingResult({ success: false, message: err.message || 'Failed to fund wallet.' })
    } finally {
      setWalletFundingLoading(false)
    }
  }, [walletAccount?.address, activeNetworkProfile, activeFaucetUrl, refreshWalletBalance, walletFundingCooldownActive, walletFundingCooldownSecs])

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

  const doStake = useCallback(async (role, amountOverride = null) => {
    const requestedAmount = amountOverride || stakeAmount
    if (!walletAccount || !requestedAmount) return
    if (walletNativeBalance !== null && parseFloat(walletNativeBalance) <= 0) {
      setStakeResult({ success: false, message: `This wallet has no ${activeChain.nativeCurrency.symbol} for gas. Use the faucet to get native testnet ETH before sending transactions.` })
      return
    }
    if (!tokenContractAddress || !stakingContractAddress) {
      setStakeResult({ success: false, message: activeNetworkProfile.id === 'anvil' ? 'Live contract addresses are unavailable. Wait for runtime config from the node or rebuild with testnet addresses.' : `No Chain Registry deployment is configured for ${activeNetworkProfile.label} yet. Set the relevant VITE_${activeNetworkProfile.shortLabel.toUpperCase()}_* contract env vars before using direct staking on this public profile.` })
      return
    }
    setStakeLoading(true)
    setStakeResult(null)
    try {
      const walletClient = walletProvider
        ? createWalletClient({ chain: activeChain, transport: custom(walletProvider) })
        : createWalletClient({ chain: activeChain, transport: http(activeRpcUrl) })
      const publicClient = createPublicClient({ chain: activeChain, transport: http(activeRpcUrl) })
      const amountWei = parseUnits(requestedAmount, 18)
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
        setStakeResult({ success: true, message: `Staked ${requestedAmount} tCREG as publisher`, tx: stakeTx })
        recordStakeTx('publisher', requestedAmount, stakeTx)
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
        const nextStepMessage = walletValidatorRegistration?.registered_with_node
          ? 'Waiting for governance approval and sync admission.'
          : 'Next step: register this wallet to a validator node identity below so the node can admit you after approval.'
        setStakeResult({ success: true, message: `Applied on-chain as validator with ${requestedAmount} tCREG. ${nextStepMessage}`, tx: stakeTx })
        recordStakeTx('validator', requestedAmount, stakeTx)
      }
      await refreshWalletBalance()
      await fetchData()
    } catch (err) {
      setStakeResult({ success: false, message: err.message })
    } finally {
      setStakeLoading(false)
    }
  }, [walletAccount, walletProvider, stakeAmount, tokenContractAddress, stakingContractAddress, refreshWalletBalance, walletValidatorRegistration, fetchData, walletNativeBalance, activeChain, activeRpcUrl, activeNetworkProfile, recordStakeTx])

  const doSponsoredStake = useCallback(async (role, amountOverride = null) => {
    const requestedAmount = amountOverride || stakeAmount
    if (!walletAccount?.address || !requestedAmount) return
    if (!tokenContractAddress || !stakingContractAddress) {
      setStakeResult({ success: false, message: activeNetworkProfile.id === 'anvil' ? 'Live contract addresses are unavailable. Wait for runtime config from the node or rebuild with testnet addresses.' : `No Chain Registry deployment is configured for ${activeNetworkProfile.label} yet. Set the relevant VITE_${activeNetworkProfile.shortLabel.toUpperCase()}_* contract env vars before using direct staking on this public profile.` })
      return
    }

    const activePolicy = role === 'publisher' ? activeSponsoredPublisherPolicy : activeSponsoredValidatorPolicy
    if (!activePolicy) {
      setStakeResult({ success: false, message: relayerPolicyError || `Sponsored ${role} actions are not available for ${activeNetworkProfile.label}.` })
      return
    }

    setSponsoredStakeLoading(true)
    setStakeResult(null)

    try {
      if (walletProvider?.request) {
        await ensureWalletChain(walletProvider)
      }

      const walletClient = createSigningWalletClient()
      const walletSigner = walletAccount.account || walletAccount.address
      const amountWei = parseUnits(requestedAmount, 18)

      const quoteResponse = await fetch(`${directRelayerUrl}/v1/relayer/quote`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          owner: walletAccount.address,
          chainId: activeChain.id,
          action: role,
          amountWei: amountWei.toString(),
          tokenContract: tokenContractAddress,
          stakingContract: stakingContractAddress,
        }),
      })
      const quote = await quoteResponse.json().catch(() => null)

      if (!quoteResponse.ok) {
        throw new Error(quote?.reason || quote?.message || `Relayer quote failed with status ${quoteResponse.status}`)
      }
      if (!quote?.allowed) {
        throw new Error(quote?.reason || `Sponsored ${role} action is not allowed by the relayer policy.`)
      }

      const permitSignature = await walletClient.signTypedData({
        account: walletSigner,
        domain: quote.permitDomain,
        types: PERMIT_TYPED_DATA_TYPES,
        primaryType: 'Permit',
        message: {
          owner: quote.permitMessage.owner,
          spender: quote.permitMessage.spender,
          value: BigInt(quote.permitMessage.value),
          nonce: BigInt(quote.permitMessage.nonce),
          deadline: BigInt(quote.permitMessage.deadline),
        },
      })

      const intentSignature = await walletClient.signTypedData({
        account: walletSigner,
        domain: quote.intentDomain,
        types: SPONSORED_STAKE_INTENT_TYPES,
        primaryType: 'SponsoredStakeIntent',
        message: {
          owner: quote.intentMessage.owner,
          tokenContract: quote.intentMessage.tokenContract,
          stakingContract: quote.intentMessage.stakingContract,
          action: Number(quote.intentMessage.action),
          amount: BigInt(quote.intentMessage.amount),
          permitNonce: BigInt(quote.intentMessage.permitNonce),
          permitDeadline: BigInt(quote.intentMessage.permitDeadline),
          relayerNonce: BigInt(quote.intentMessage.relayerNonce),
          expiresAt: BigInt(quote.intentMessage.expiresAt),
        },
      })

      const sponsorResponse = await fetch(`${directRelayerUrl}/v1/relayer/sponsor`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          action: role,
          permitMessage: quote.permitMessage,
          intentMessage: quote.intentMessage,
          permitSignature,
          intentSignature,
        }),
      })
      const sponsored = await sponsorResponse.json().catch(() => null)

      if (!sponsorResponse.ok || !sponsored?.success) {
        throw new Error(sponsored?.message || `Relayer sponsor request failed with status ${sponsorResponse.status}`)
      }

      const submittedMessage = role === 'publisher'
        ? `Sponsored publisher stake submitted for ${requestedAmount} tCREG. Waiting for confirmation...`
        : `Sponsored validator application submitted for ${requestedAmount} tCREG. Waiting for confirmation...`
      setStakeResult({ success: true, message: sponsored.message || submittedMessage, tx: sponsored.txHash || null })

      if (sponsored.requestId) {
        await pollSponsoredRequest(sponsored.requestId, role, requestedAmount, sponsored.txHash || null)
      } else {
        await refreshWalletBalance()
        await fetchData()
      }
    } catch (err) {
      setStakeResult({ success: false, message: err.message || `Sponsored ${role} action failed.` })
    } finally {
      setSponsoredStakeLoading(false)
    }
  }, [walletAccount, walletProvider, stakeAmount, tokenContractAddress, stakingContractAddress, activeNetworkProfile, activeSponsoredPublisherPolicy, activeSponsoredValidatorPolicy, relayerPolicyError, activeChain, directRelayerUrl, createSigningWalletClient, ensureWalletChain, pollSponsoredRequest, refreshWalletBalance, fetchData])

  const registerValidatorIdentity = useCallback(async () => {
    if (!walletAccount?.address) return

    const nodeId = validatorIdentityForm.nodeId.trim()
    const alias = validatorIdentityForm.alias.trim()
    const ed25519Pubkey = validatorIdentityForm.ed25519Pubkey.trim().replace(/^0x/i, '').toLowerCase()

    if (!nodeId) {
      setValidatorRegistrationResult({ success: false, message: 'Node ID is required.' })
      return
    }
    if (!/^[a-f0-9]{64}$/i.test(ed25519Pubkey)) {
      setValidatorRegistrationResult({ success: false, message: 'Ed25519 pubkey must be 64 hex characters.' })
      return
    }

    setValidatorRegistrationLoading(true)
    setValidatorRegistrationResult(null)
    try {
      const res = await fetch(`${API_BASE}/v1/validators/register`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          evm_address: walletAccount.address,
          node_id: nodeId,
          ed25519_pubkey: ed25519Pubkey,
          alias: alias || nodeId,
        }),
      })

      const body = await res.json().catch(() => null)
      if (!res.ok) {
        setValidatorRegistrationResult({ success: false, message: body?.error || 'Validator registration failed.' })
        return
      }

      setValidatorRegistrationResult({
        success: true,
        message: 'Validator identity registered with the node. Once governance approves the on-chain application, the sync loop will admit it into the live validator set.',
      })
      setValidatorRegistrations((current) => {
        const next = current.filter((registration) => registration?.identity?.evm_address?.toLowerCase() !== walletAccount.address.toLowerCase())
        next.push(body)
        return next
      })
      await fetchData()
    } catch (err) {
      setValidatorRegistrationResult({ success: false, message: err.message || 'Validator registration failed.' })
    } finally {
      setValidatorRegistrationLoading(false)
    }
  }, [walletAccount, validatorIdentityForm, fetchData])

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

    // Build a per-field error map so the UI can highlight each invalid input.
    const { name, version, content_hash, ipfs_cid, publisher_pubkey, signature } = publishForm
    const errs = {}
    if (!name || !name.trim())
      errs.name = 'Package name is required'
    if (!version || !/^\d+\.\d+(\.\d+)?/.test(version))
      errs.version = 'Must be valid semver (e.g. 1.0.0)'
    if (!content_hash || !/^[a-f0-9]{64}$/i.test(content_hash))
      errs.content_hash = '64-char hex SHA-256 required'
    if (!ipfs_cid || !/^(Qm[a-zA-Z0-9]{44}|bafy[a-zA-Z0-9]+)$/.test(ipfs_cid))
      errs.ipfs_cid = 'Expected Qm… (v0) or bafy… (v1) CID'
    if (!publisher_pubkey || !/^[a-f0-9]{64}$/i.test(publisher_pubkey))
      errs.publisher_pubkey = 'Ed25519 pubkey must be 64 hex chars'
    if (!signature || !/^[a-f0-9]{128}$/i.test(signature))
      errs.signature = 'Ed25519 signature must be 128 hex chars'

    if (Object.keys(errs).length > 0) {
      setPublishErrors(errs)
      setPublishStatus({ ok: false, msg: 'Please fix the highlighted fields' })
      return
    }
    setPublishErrors({})

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
        setPublishErrors({})
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
  }, [publishForm, fetchPackageList, setPublishErrors])

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

      {/* SSE Reconnect Banner (WEB-H02) */}
      {!sseConnected && (
        <div style={{
          background: '#7f1d1d',
          borderBottom: '1px solid #ef4444',
          color: '#fca5a5',
          padding: '6px 16px',
          display: 'flex',
          alignItems: 'center',
          gap: '8px',
          fontSize: '0.85rem',
          fontWeight: 500,
        }}>
          <span style={{ color: '#ef4444', fontSize: '1rem' }}>⚠</span>
          {sseReconnectIn > 0
            ? `Live event stream disconnected — reconnecting in ${sseReconnectIn}s…`
            : 'Live event stream disconnected — reconnecting…'}
        </div>
      )}

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
              <div className="stat-value">{(p2pStatus.peers || []).length}</div>
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
              {view === 'wallet' && 'Wallet & Stake'}
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
                            {(() => {
                              const rep = node.reputation || 0
                              const color = rep >= 80 ? '#22c55e' : rep >= 60 ? '#eab308' : rep >= 40 ? '#f97316' : '#ef4444'
                              const label = rep >= 80 ? 'High' : rep >= 60 ? 'Good' : rep >= 40 ? 'Fair' : 'Low'
                              return (
                                <div className="rep-bar" title={`Reputation: ${rep}/100 (${label})`}>
                                  <div className="rep-track">
                                    <div className="rep-fill" style={{ width: `${rep}%`, background: color }} />
                                  </div>
                                  <span className="rep-value" style={{ color }}>{rep}</span>
                                </div>
                              )
                            })()}
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
                      {/* Ecosystem / Name / Version row */}
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
                        <div>
                          <input
                            className="search-input"
                            placeholder="Package name"
                            value={publishForm.name}
                            style={publishErrors.name ? { borderColor: '#ef4444', outlineColor: '#ef4444' } : {}}
                            onChange={e => { setPublishForm(f => ({ ...f, name: e.target.value })); setPublishErrors(ev => ({ ...ev, name: null })) }}
                          />
                          {publishErrors.name && <p style={{ color: '#ef4444', fontSize: '0.75rem', margin: '2px 0 0' }}>{publishErrors.name}</p>}
                        </div>
                        <div>
                          <input
                            className="search-input"
                            placeholder="Version (e.g. 1.0.0)"
                            value={publishForm.version}
                            style={publishErrors.version ? { borderColor: '#ef4444', outlineColor: '#ef4444' } : {}}
                            onChange={e => { setPublishForm(f => ({ ...f, version: e.target.value })); setPublishErrors(ev => ({ ...ev, version: null })) }}
                          />
                          {publishErrors.version && <p style={{ color: '#ef4444', fontSize: '0.75rem', margin: '2px 0 0' }}>{publishErrors.version}</p>}
                        </div>
                      </div>
                      {/* IPFS CID */}
                      <div>
                        <input
                          className="search-input"
                          placeholder="IPFS CID (bafy… or Qm…)"
                          value={publishForm.ipfs_cid}
                          style={publishErrors.ipfs_cid ? { borderColor: '#ef4444', outlineColor: '#ef4444' } : {}}
                          onChange={e => { setPublishForm(f => ({ ...f, ipfs_cid: e.target.value })); setPublishErrors(ev => ({ ...ev, ipfs_cid: null })) }}
                        />
                        {publishErrors.ipfs_cid && <p style={{ color: '#ef4444', fontSize: '0.75rem', margin: '2px 0 0' }}>{publishErrors.ipfs_cid}</p>}
                      </div>
                      {/* Content hash */}
                      <div>
                        <input
                          className="search-input"
                          placeholder="Content hash (SHA-256, 64 hex chars)"
                          value={publishForm.content_hash}
                          style={publishErrors.content_hash ? { borderColor: '#ef4444', outlineColor: '#ef4444' } : {}}
                          onChange={e => { setPublishForm(f => ({ ...f, content_hash: e.target.value })); setPublishErrors(ev => ({ ...ev, content_hash: null })) }}
                        />
                        {publishErrors.content_hash && <p style={{ color: '#ef4444', fontSize: '0.75rem', margin: '2px 0 0' }}>{publishErrors.content_hash}</p>}
                      </div>
                      {/* Publisher pubkey */}
                      <div>
                        <input
                          className="search-input"
                          placeholder="Publisher public key (64 hex chars)"
                          value={publishForm.publisher_pubkey}
                          style={publishErrors.publisher_pubkey ? { borderColor: '#ef4444', outlineColor: '#ef4444' } : {}}
                          onChange={e => { setPublishForm(f => ({ ...f, publisher_pubkey: e.target.value })); setPublishErrors(ev => ({ ...ev, publisher_pubkey: null })) }}
                        />
                        {publishErrors.publisher_pubkey && <p style={{ color: '#ef4444', fontSize: '0.75rem', margin: '2px 0 0' }}>{publishErrors.publisher_pubkey}</p>}
                      </div>
                      {/* Signature */}
                      <div>
                        <input
                          className="search-input"
                          placeholder="Ed25519 signature (128 hex chars)"
                          value={publishForm.signature}
                          style={publishErrors.signature ? { borderColor: '#ef4444', outlineColor: '#ef4444' } : {}}
                          onChange={e => { setPublishForm(f => ({ ...f, signature: e.target.value })); setPublishErrors(ev => ({ ...ev, signature: null })) }}
                        />
                        {publishErrors.signature && <p style={{ color: '#ef4444', fontSize: '0.75rem', margin: '2px 0 0' }}>{publishErrors.signature}</p>}
                      </div>
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
                {(packageList.packages || []).length > 0 && (
                  <div className="detail-section" style={{ marginBottom: 'var(--space-4)' }}>
                    <div className="detail-section-title">On-chain Packages ({packageList.total} total)</div>
                    <div className="search-box" style={{ marginBottom: 'var(--space-2)' }}>
                      <span className="search-icon">🔍</span>
                      <input
                        type="text"
                        className="search-input"
                        placeholder="Filter packages by name, ecosystem, status..."
                        value={packageFilterText}
                        onChange={(e) => setPackageFilterText(e.target.value)}
                      />
                    </div>
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
                          {(packageList.packages || []).filter(pkg => {
                            if (!packageFilterText) return true
                            const q = packageFilterText.toLowerCase()
                            return (pkg.canonical || '').toLowerCase().includes(q)
                              || (pkg.name || '').toLowerCase().includes(q)
                              || (pkg.ecosystem || '').toLowerCase().includes(q)
                              || (pkg.status || '').toLowerCase().includes(q)
                              || (pkg.publisher || '').toLowerCase().includes(q)
                          }).map((pkg, idx) => (
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
              <div style={{ padding: 'var(--space-4)', maxWidth: 640, margin: '0 auto' }}>

                {/* ── Network Selector (compact inline) ─────────────────── */}
                <div style={{ display: 'flex', gap: '8px', marginBottom: '8px' }}>
                  {Object.values(networkProfiles).map((profile) => (
                    <button
                      key={profile.id}
                      type="button"
                      onClick={() => selectNetworkProfile(profile.id)}
                      style={{
                        flex: 1,
                        padding: '10px 12px',
                        borderRadius: '10px',
                        border: `1.5px solid ${activeNetworkProfile.id === profile.id ? 'rgba(99,102,241,0.5)' : 'var(--border)'}`,
                        background: activeNetworkProfile.id === profile.id ? 'rgba(99,102,241,0.1)' : 'var(--surface)',
                        color: 'var(--text-primary)',
                        cursor: 'pointer',
                        textAlign: 'center',
                        fontSize: '0.85rem',
                        fontWeight: activeNetworkProfile.id === profile.id ? 600 : 400,
                      }}
                    >
                      {profile.shortLabel}
                      <div style={{ fontSize: '0.7rem', color: 'var(--text-tertiary)', marginTop: '2px' }}>
                        {profile.chain.id}
                      </div>
                    </button>
                  ))}
                </div>
                {activeNetworkProfile.id !== 'anvil' && (
                  <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 'var(--space-4)' }}>
                    <button
                      type="button"
                      onClick={() => {
                        try { window.localStorage.removeItem('creg.walletNetworkProfile') } catch (_) { /* ignore */ }
                        selectNetworkProfile('anvil')
                      }}
                      style={{
                        background: 'transparent', border: 'none', padding: '2px 0',
                        color: 'var(--text-tertiary)', fontSize: '0.75rem',
                        cursor: 'pointer', textDecoration: 'underline',
                      }}
                    >
                      Reset to Local Anvil
                    </button>
                  </div>
                )}

                {/* ── Not Connected: Connect Wallet ─────────────────────── */}
                {!walletAccount ? (
                  <div className="detail-panel">
                    <div className="detail-header">
                      <span className="detail-title">Connect Wallet</span>
                    </div>
                    <div className="detail-content" style={{ display: 'grid', gap: '10px' }}>
                      {eip6963Providers.length > 0 ? (
                        eip6963Providers.map((p, i) => (
                          <button
                            key={p.info?.uuid || i}
                            className="wallet-action-btn wallet-action-primary"
                            onClick={() => connectEip6963(p)}
                            style={{ width: '100%', display: 'flex', alignItems: 'center', gap: '8px', justifyContent: 'center' }}
                          >
                            {p.info?.icon && <img src={p.info.icon} alt="" style={{ width: 20, height: 20 }} />}
                            {p.info?.name || 'Wallet'}
                          </button>
                        ))
                      ) : (
                        <button className="wallet-action-btn wallet-action-primary" onClick={connectMetaMask} style={{ width: '100%' }}>
                          Connect MetaMask
                        </button>
                      )}
                      <button className="wallet-action-btn wallet-action-secondary" onClick={connectWalletConnect} style={{ width: '100%' }}>
                        WalletConnect
                      </button>
                      {PRIVATE_KEY_WALLET_ENABLED && (
                        <div style={{ display: 'flex', gap: '8px' }}>
                          <input
                            type="password"
                            className="wallet-input"
                            placeholder="Private key (dev only)"
                            autoComplete="off"
                            spellCheck="false"
                            value={walletKeyInput}
                            onChange={(e) => setWalletKeyInput(e.target.value)}
                            onKeyDown={(e) => e.key === 'Enter' && walletKeyInput && connectWallet(walletKeyInput)}
                            style={{ flex: 1, marginBottom: 0 }}
                          />
                          <button className="wallet-action-btn wallet-action-primary" onClick={() => connectWallet(walletKeyInput)} disabled={!walletKeyInput} style={{ whiteSpace: 'nowrap' }}>
                            Connect
                          </button>
                        </div>
                      )}
                      <div style={{ color: 'var(--text-tertiary)', fontSize: '0.8rem', textAlign: 'center' }}>
                        Connects to {activeNetworkProfile.label} (Chain {activeChain.id})
                      </div>
                    </div>
                  </div>
                ) : (
                  <>
                    {/* ── Connected: Account & Balances ──────────────────── */}
                    <div className="detail-panel" style={{ marginBottom: 'var(--space-3)' }}>
                      <div className="detail-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                        <span className="detail-title">
                          <CopyButton text={walletAccount.address} label="address" />
                        </span>
                        <span style={{ fontSize: '0.75rem', color: 'var(--text-tertiary)' }}>
                          {walletAccount.providerName || walletAccount.type} · {activeNetworkProfile.shortLabel}
                        </span>
                      </div>
                      <div className="detail-content">
                        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px', marginBottom: '12px' }}>
                          <div className="wallet-balance-display">
                            <span className="wallet-balance-value">{walletBalance || '...'}</span>
                            <span className="wallet-balance-label">tCREG</span>
                          </div>
                          <div className="wallet-balance-display" style={{ background: 'rgba(14,165,233,0.08)', border: '1px solid rgba(14,165,233,0.24)' }}>
                            <span className="wallet-balance-value">{walletNativeBalance || '...'}</span>
                            <span className="wallet-balance-label">ETH Gas</span>
                          </div>
                        </div>
                        {walletRpcOffline && (
                          <div style={{
                            padding: '8px 10px', marginBottom: '10px', borderRadius: '8px',
                            background: 'rgba(248,113,113,0.08)', border: '1px solid rgba(248,113,113,0.25)',
                            color: '#f87171', fontSize: '0.78rem', lineHeight: 1.4,
                          }}>
                            RPC unreachable at {activeRpcUrl}. Balances may be stale. Polling backed off to 60s.
                          </div>
                        )}
                        <div style={{ display: 'flex', gap: '8px', flexWrap: 'wrap' }}>
                          <button className="wallet-inline-action" onClick={refreshWalletBalance}>Refresh</button>
                          <button className="wallet-inline-action" onClick={() => setView('packages')}>Packages</button>
                          <button className="wallet-inline-action" onClick={() => setView('validators')}>Validators</button>
                          <button className="wallet-inline-action" onClick={disconnectWallet} style={{ marginLeft: 'auto', color: '#f87171' }}>Disconnect</button>
                        </div>
                      </div>
                    </div>

                    {/* ── Faucet Funding ─────────────────────────────────── */}
                    <div className="detail-panel" style={{ marginBottom: 'var(--space-3)' }}>
                      <div className="detail-header">
                        <span className="detail-title">Faucet</span>
                      </div>
                      <div className="detail-content">
                        {(walletNativeBalance !== null && parseFloat(walletNativeBalance) === 0) && (
                          <div style={{ color: '#fbbf24', fontSize: '0.82rem', marginBottom: '10px' }}>
                            Your wallet has no ETH for gas. Fund it before staking or publishing.
                          </div>
                        )}
                        <div style={{ display: 'flex', gap: '8px' }}>
                          <button
                            className="wallet-action-btn wallet-action-primary"
                            onClick={fundConnectedWallet}
                            disabled={walletFundingLoading || walletFundingCooldownActive}
                            style={{ flex: 1 }}
                          >
                            {walletFundingButtonLabel}
                          </button>
                          <a
                            className="wallet-action-btn wallet-action-secondary"
                            href={activeFaucetUrl}
                            target="_blank"
                            rel="noopener noreferrer"
                            style={{ textDecoration: 'none', textAlign: 'center', display: 'flex', alignItems: 'center', justifyContent: 'center' }}
                          >
                            Open Faucet
                          </a>
                        </div>
                        {walletFundingResult && (
                          <div className={`wallet-result ${walletFundingResult.success ? 'success' : 'error'}`} style={{ marginTop: '10px' }}>
                            {walletFundingResult.message}
                            {walletFundingResult.tokenTxHash && <div className="wallet-tx">Token: {truncateHash(walletFundingResult.tokenTxHash, 10, 6)}</div>}
                            {walletFundingResult.nativeTxHash && <div className="wallet-tx">Gas: {truncateHash(walletFundingResult.nativeTxHash, 10, 6)}</div>}
                          </div>
                        )}
                      </div>
                    </div>

                    {/* ── Staking ────────────────────────────────────────── */}
                    <div className="detail-panel" style={{ marginBottom: 'var(--space-3)' }}>
                      <div className="detail-header">
                        <span className="detail-title">Staking</span>
                      </div>
                      <div className="detail-content">
                        {(!tokenContractAddress || !stakingContractAddress) && (
                          <div className="wallet-result warning" style={{ marginBottom: '10px' }}>
                            {activeNetworkProfile.id === 'anvil'
                              ? 'Contract addresses are loading from the node...'
                              : `Configure VITE_${activeNetworkProfile.shortLabel.toUpperCase()}_* env vars to enable staking.`}
                          </div>
                        )}
                        <input
                          type="number"
                          className="wallet-input"
                          placeholder="Amount in tCREG"
                          value={stakeAmount}
                          onChange={(e) => setStakeAmount(e.target.value)}
                          style={{ marginBottom: '10px' }}
                        />
                        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '8px', marginBottom: '8px' }}>
                          <button
                            className="wallet-action-btn wallet-action-primary"
                            onClick={() => doStake('publisher')}
                            disabled={stakeLoading || !stakeAmount}
                            style={{ width: '100%' }}
                          >
                            {stakeLoading ? 'Staking...' : 'Stake as Publisher'}
                          </button>
                          <button
                            className="wallet-action-btn wallet-action-secondary"
                            onClick={() => doStake('validator', stakeAmount || '100')}
                            disabled={stakeLoading}
                            style={{ width: '100%' }}
                          >
                            {stakeLoading ? 'Applying...' : 'Apply as Validator'}
                          </button>
                        </div>
                        {(activeSponsoredPublisherPolicy || activeSponsoredValidatorPolicy) && (
                          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '8px' }}>
                            <button
                              className="wallet-action-btn wallet-action-secondary"
                              onClick={() => doSponsoredStake('publisher')}
                              disabled={sponsoredStakeLoading || !stakeAmount || !activeSponsoredPublisherPolicy}
                              style={{ width: '100%', fontSize: '0.8rem' }}
                            >
                              Sponsored Publisher
                            </button>
                            <button
                              className="wallet-action-btn wallet-action-secondary"
                              onClick={() => doSponsoredStake('validator', stakeAmount || '100')}
                              disabled={sponsoredStakeLoading || !activeSponsoredValidatorPolicy}
                              style={{ width: '100%', fontSize: '0.8rem' }}
                            >
                              Sponsored Validator
                            </button>
                          </div>
                        )}
                        {stakeResult && (
                          <div className={`wallet-result ${stakeResult.success ? 'success' : 'error'}`} style={{ marginTop: '10px' }}>
                            {stakeResult.message}
                            {stakeResult.tx && <div className="wallet-tx">Tx: {truncateHash(stakeResult.tx, 10, 6)}</div>}
                          </div>
                        )}
                      </div>
                    </div>

                    {/* ── Validator Identity (collapsible) ──────────────── */}
                    <details className="detail-panel" style={{ marginBottom: 'var(--space-3)' }}>
                      <summary className="detail-header" style={{ cursor: 'pointer', userSelect: 'none' }}>
                        <span className="detail-title">Validator Identity</span>
                        {walletValidatorRegistration && (
                          <span className={`badge badge-${walletValidatorRegistration.active ? 'success' : 'warning'}`} style={{ marginLeft: '8px' }}>
                            {walletValidatorRegistration.active ? 'active' : walletValidatorRegistration.status || 'pending'}
                          </span>
                        )}
                      </summary>
                      <div className="detail-content">
                        {detectedValidatorNodes.length > 0 && (
                          <div style={{ marginBottom: '12px' }}>
                            <div style={{ fontSize: '0.8rem', color: 'var(--text-secondary)', marginBottom: '6px' }}>Detected nodes:</div>
                            {detectedValidatorNodes.map((node) => (
                              <div key={node.id} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '8px 10px', borderRadius: '8px', background: 'var(--surface)', marginBottom: '4px' }}>
                                <div>
                                  <span style={{ fontWeight: 600, fontSize: '0.85rem' }}>{node.alias || node.id}</span>
                                  <StatusBadge status={node.status || 'online'} />
                                </div>
                                <button className="wallet-inline-action" onClick={() => adoptNodeIdentity(node)}>Use</button>
                              </div>
                            ))}
                          </div>
                        )}
                        <input type="text" className="wallet-input" placeholder="Node ID (e.g. node-2)" value={validatorIdentityForm.nodeId} onChange={(e) => setValidatorIdentityForm((c) => ({ ...c, nodeId: e.target.value }))} style={{ marginBottom: '6px' }} />
                        <input type="text" className="wallet-input" placeholder="Ed25519 pubkey (64 hex)" value={validatorIdentityForm.ed25519Pubkey} onChange={(e) => setValidatorIdentityForm((c) => ({ ...c, ed25519Pubkey: e.target.value }))} spellCheck="false" style={{ marginBottom: '6px' }} />
                        <input type="text" className="wallet-input" placeholder="Alias (optional)" value={validatorIdentityForm.alias} onChange={(e) => setValidatorIdentityForm((c) => ({ ...c, alias: e.target.value }))} style={{ marginBottom: '10px' }} />
                        <button className="wallet-action-btn wallet-action-primary" onClick={registerValidatorIdentity} disabled={validatorRegistrationLoading || !validatorIdentityForm.nodeId || !validatorIdentityForm.ed25519Pubkey} style={{ width: '100%' }}>
                          {validatorRegistrationLoading ? 'Saving...' : walletValidatorRegistration ? 'Update Identity' : 'Register Identity'}
                        </button>
                        {validatorRegistrationResult && (
                          <div className={`wallet-result ${validatorRegistrationResult.success ? 'success' : 'error'}`} style={{ marginTop: '8px' }}>
                            {validatorRegistrationResult.message}
                          </div>
                        )}
                        {walletValidatorRegistration && (
                          <div style={{ display: 'grid', gap: '4px', marginTop: '12px' }}>
                            {validatorLifecycle.map((step) => (
                              <div key={step.key} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '6px 10px', borderRadius: '8px', background: step.complete ? 'rgba(16,185,129,0.1)' : 'rgba(148,163,184,0.08)', fontSize: '0.82rem' }}>
                                <span>{step.complete ? '✓' : '○'} {step.label}</span>
                                <span style={{ fontSize: '0.7rem', color: step.complete ? '#34d399' : 'var(--text-tertiary)', textTransform: 'uppercase' }}>{step.complete ? 'done' : 'waiting'}</span>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    </details>

                    {/* ── Transaction History (compact) ──────────────────── */}
                    {stakeTxHistory.length > 0 && (
                      <div className="detail-panel">
                        <div className="detail-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                          <span className="detail-title">Recent Transactions</span>
                          <button
                            style={{ background: 'none', border: 'none', color: 'var(--text-muted)', cursor: 'pointer', fontSize: '0.75rem' }}
                            onClick={() => { setStakeTxHistory([]); try { window.localStorage.removeItem('creg.stakeTxHistory') } catch {} }}
                          >
                            Clear
                          </button>
                        </div>
                        <div className="detail-content" style={{ display: 'grid', gap: '4px' }}>
                          {stakeTxHistory.slice(0, 5).map((entry, i) => (
                            <div key={i} style={{ display: 'flex', justifyContent: 'space-between', fontSize: '0.82rem', padding: '6px 0', borderBottom: '1px solid var(--border)' }}>
                              <span style={{ textTransform: 'capitalize' }}>{entry.type} {entry.amount} tCREG</span>
                              <span style={{ color: 'var(--text-muted)' }}>{timeAgo(entry.at)}</span>
                            </div>
                          ))}
                        </div>
                      </div>
                    )}
                  </>
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
                  <div className="detail-section-title">Connected Peers ({(p2pStatus.peers || []).length})</div>
                  {(p2pStatus.peers || []).length === 0 ? (
                    <EmptyState 
                      icon="🌐" 
                      title="No peers connected" 
                      description="Searching for peers via DHT..."
                    />
                  ) : (
                    <div className="peer-list">
                      {(p2pStatus.peers || []).map((peer, idx) => (
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
                    {(() => {
                      const rep = selectedValidator.reputation || 0
                      const color = rep >= 80 ? '#22c55e' : rep >= 60 ? '#eab308' : rep >= 40 ? '#f97316' : '#ef4444'
                      const band = rep >= 80 ? 'High' : rep >= 60 ? 'Good' : rep >= 40 ? 'Fair' : 'Low'
                      return (
                        <span className="detail-value" style={{ color }} title={`${band} reputation`}>
                          {rep}/100 <span style={{ fontSize: '0.75rem', opacity: 0.8 }}>({band})</span>
                        </span>
                      )
                    })()}
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
