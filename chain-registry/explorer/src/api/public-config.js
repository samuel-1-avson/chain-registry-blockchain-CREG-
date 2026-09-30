const configByChain = new Map()

const SPEC_PATH_BY_CHAIN = new Map([
  ['creg-phase-c-testnet-2026-09-26-r7', '/phase-c/r7/chain-spec.json'],
])

function validContractAddress(value) {
  return typeof value === 'string'
    && /^0x[0-9a-fA-F]{40}$/.test(value)
    && !/^0x0{40}$/i.test(value)
}

export async function readPublicChainConfig(stats) {
  if (!stats?.spec_signature_verified) {
    throw new Error('The node has not verified its signed chain specification.')
  }
  if (typeof stats.chain_id !== 'string' || !stats.chain_id || typeof stats.genesis_hash !== 'string' || !stats.genesis_hash) {
    throw new Error('The live node did not provide a complete chain identity.')
  }

  const cacheKey = `${stats.chain_id}:${stats.genesis_hash}`
  if (configByChain.has(cacheKey)) return configByChain.get(cacheKey)

  const specPath = SPEC_PATH_BY_CHAIN.get(stats.chain_id) || '/chain-spec.json'
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 8000)
  let response
  try {
    response = await fetch(specPath, {
      cache: 'no-store',
      credentials: 'same-origin',
      headers: { Accept: 'application/json' },
      signal: controller.signal,
    })
  } finally {
    clearTimeout(timeout)
  }

  if (!response.ok) throw new Error(`Signed chain specification unavailable (HTTP ${response.status}).`)
  const spec = await response.json()
  if (spec?.chain_id !== stats.chain_id) {
    throw new Error(`Chain specification ${spec?.chain_id || '(missing chain id)'} does not match live node ${stats.chain_id}.`)
  }
  if (spec?.l1?.chain_id !== 11155111) {
    throw new Error('The signed chain specification does not identify Sepolia as its L1.')
  }

  const contracts = spec.contracts || {}
  const config = {
    tokenContract: contracts.creg_token,
    stakingContract: contracts.staking,
    registryAddress: contracts.registry,
    chainId: spec.chain_id,
    genesisHash: stats.genesis_hash,
  }
  for (const [name, address] of Object.entries({
    token: config.tokenContract,
    staking: config.stakingContract,
    registry: config.registryAddress,
  })) {
    if (!validContractAddress(address)) throw new Error(`The signed chain specification has no valid ${name} contract address.`)
  }

  configByChain.set(cacheKey, config)
  return config
}
