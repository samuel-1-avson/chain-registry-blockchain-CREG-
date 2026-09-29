// Read-only proposal for the requested 20m tCREG faucet custody target.
// This file never loads a signing key or broadcasts a transaction.
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { Interface, keccak256 } = require('../../scripts/evm-tool/node_modules/ethers')
const manifest = JSON.parse(await fs.readFile(new URL('./deployment-manifest.json', import.meta.url)))
const rpcUrl = 'https://api.testnet.cregnet.dev/sepolia'
const faucet = '0x721C8dBD8208CA9dBdcC12AE9339225458608B81'
const unit = 10n ** 18n
const target = 20_000_000n * unit
const tokenAbi = new Interface([
  'function balanceOf(address) view returns (uint256)',
  'function totalSupply() view returns (uint256)',
  'function MAX_SUPPLY() view returns (uint256)',
  'function owner() view returns (address)',
  'function treasury() view returns (address)',
  'function mint(address,uint256)',
  'function transfer(address,uint256) returns (bool)',
])
const governanceAbi = new Interface([
  'function threshold() view returns (uint256)',
  'function isPaused() view returns (bool)',
  'function submit(address,bytes,string) returns (uint256)',
])
let rpcId = 0

async function rpc(method, params) {
  const response = await fetch(rpcUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: ++rpcId, method, params }),
    signal: AbortSignal.timeout(20000),
  })
  assert.equal(response.status, 200, `${method} HTTP status`)
  const body = await response.json()
  if (body.error) throw new Error(`${method}: ${body.error.message}`)
  assert.ok(body.result !== undefined && body.result !== null, `${method} returned no result`)
  return body.result
}

async function read(method, args, blockTag) {
  const data = tokenAbi.encodeFunctionData(method, args)
  const result = await rpc('eth_call', [{ to: manifest.cregToken, data }, blockTag])
  return tokenAbi.decodeFunctionResult(method, result)[0]
}

function format(raw) {
  const fraction = (raw % unit).toString().padStart(18, '0').replace(/0+$/, '')
  return (raw / unit).toString() + (fraction ? '.' + fraction : '')
}

assert.equal(BigInt(await rpc('eth_chainId', [])), 11155111n)
const blockTag = await rpc('eth_blockNumber', [])
const code = await rpc('eth_getCode', [manifest.cregToken, blockTag])
assert.notEqual(code, '0x', 'token contract is absent')
const [supply, cap, treasuryBalance, faucetBalance, stakingBalance, owner, treasury] = await Promise.all([
  read('totalSupply', [], blockTag),
  read('MAX_SUPPLY', [], blockTag),
  read('balanceOf', [manifest.deployer], blockTag),
  read('balanceOf', [faucet], blockTag),
  read('balanceOf', [manifest.staking], blockTag),
  read('owner', [], blockTag),
  read('treasury', [], blockTag),
])
assert.equal(owner.toLowerCase(), manifest.governance.toLowerCase(), 'governance owner changed')
assert.equal(treasury.toLowerCase(), manifest.deployer.toLowerCase(), 'treasury authority changed')
assert.equal(supply, target, 'the current total supply is no longer 20m; recompute the proposal')
assert.ok(treasuryBalance > 0n && faucetBalance < target)
const shortfall = target - faucetBalance - treasuryBalance
assert.ok(shortfall >= 0n, 'the known treasury and faucet balances exceed the target')
const directMintAmount = target - faucetBalance
assert.ok(supply + directMintAmount <= cap, 'direct faucet mint exceeds the contract cap')
const governanceReads = await Promise.all(['threshold', 'isPaused'].map(async method => {
  const result = await rpc('eth_call', [{
    to: manifest.governance,
    data: governanceAbi.encodeFunctionData(method, []),
  }, blockTag])
  return governanceAbi.decodeFunctionResult(method, result)[0]
}))
const [governanceThreshold, governancePaused] = governanceReads
assert.ok(governanceThreshold > 0n, 'governance has no approval threshold')
const mintCallData = tokenAbi.encodeFunctionData('mint', [faucet, shortfall])
const mintDescription = `Mint ${format(shortfall)} tCREG to faucet to reach ${format(target)} faucet custody after treasury transfer; total outstanding becomes ${format(supply + shortfall)} tCREG`
const directMintCallData = tokenAbi.encodeFunctionData('mint', [faucet, directMintAmount])
const directMintDescription = `Mint ${format(directMintAmount)} tCREG to faucet to reach ${format(target)} faucet custody; total outstanding becomes ${format(supply + directMintAmount)} tCREG`

const proposal = {
  status: 'prepared_read_only_not_approved_not_signed_not_broadcast',
  prepared_at_utc: new Date().toISOString(),
  chain_id: 11155111,
  snapshot_block: Number(BigInt(blockTag)),
  rpc_url: rpcUrl,
  token_contract: manifest.cregToken,
  token_code_keccak256: keccak256(code),
  token_owner: owner,
  governance_approval_threshold: governanceThreshold.toString(),
  governance_paused: governancePaused,
  requested_faucet_target_tcreg: format(target),
  current_total_supply_tcreg: format(supply),
  contract_cap_tcreg: format(cap),
  current_treasury_tcreg: format(treasuryBalance),
  current_faucet_tcreg: format(faucetBalance),
  other_holder_total_tcreg: format(supply - treasuryBalance - faucetBalance),
  staking_tcreg: format(stakingBalance),
  maximum_treasury_transfer: {
    from: treasury,
    to: manifest.cregToken,
    recipient: faucet,
    amount_tcreg: format(treasuryBalance),
    amount_wei: treasuryBalance.toString(),
    calldata: tokenAbi.encodeFunctionData('transfer', [faucet, treasuryBalance]),
    value_wei: '0',
    note: 'This is all currently available treasury tCREG; it is not a 20m transfer.',
  },
  faucet_balance_after_maximum_treasury_transfer_tcreg: format(faucetBalance + treasuryBalance),
  remaining_to_reach_20m_faucet_balance_tcreg: format(shortfall),
  direct_governance_mint_without_treasury_transfer: {
    governance_contract: manifest.governance,
    target: manifest.cregToken,
    recipient: faucet,
    amount_tcreg: format(directMintAmount),
    amount_wei: directMintAmount.toString(),
    token_mint_calldata: directMintCallData,
    governance_submit_calldata: governanceAbi.encodeFunctionData('submit', [manifest.cregToken, directMintCallData, directMintDescription]),
    proposal_description: directMintDescription,
    projected_total_supply_tcreg: format(supply + directMintAmount),
    remaining_cap_after_tcreg: format(cap - supply - directMintAmount),
    status: 'optional_not_submitted_not_approved',
  },
  optional_governance_mint_shortfall: {
    governance_contract: manifest.governance,
    target: manifest.cregToken,
    recipient: faucet,
    amount_tcreg: format(shortfall),
    amount_wei: shortfall.toString(),
    token_mint_calldata: mintCallData,
    governance_submit_calldata: governanceAbi.encodeFunctionData('submit', [manifest.cregToken, mintCallData, mintDescription]),
    proposal_description: mintDescription,
    projected_total_supply_tcreg: format(supply + shortfall),
    status: 'optional_not_submitted_not_approved',
  },
  supply_preserving_completion: 'The remaining holder balances must be transferred voluntarily to the faucet. A treasury signer cannot seize them; staking deposits may require withdrawal first.',
  alternative_completion: 'Governance could mint the shortfall directly to the faucet, but this increases total outstanding supply above the current 20m. This is a separate policy and signer decision.',
  requirements_before_any_execution: [
    'Choose and approve the exact faucet custody and allocation policy, given the hot wallet would hold nearly the full current supply.',
    'Re-read Sepolia chain ID, token code, owner, treasury, total supply, all balances, gas, and pending nonce at execution time.',
    'Review exact transaction calldata, recipient, signer custody, gas budget, claim limits, and rollback constraints.',
    'Use a separate explicit authorization for each on-chain transfer or mint.',
  ],
  requests_mocked: false,
  on_chain_transaction_sent: false,
}

const output = process.argv[2]
if (!output) throw new Error('Provide an output JSON path')
await fs.mkdir(path.dirname(output), { recursive: true })
await fs.writeFile(output, JSON.stringify(proposal, null, 2) + '\n')
console.log(JSON.stringify({
  status: proposal.status,
  snapshot_block: proposal.snapshot_block,
  current_treasury_tcreg: proposal.current_treasury_tcreg,
  current_faucet_tcreg: proposal.current_faucet_tcreg,
  faucet_balance_after_maximum_treasury_transfer_tcreg: proposal.faucet_balance_after_maximum_treasury_transfer_tcreg,
  remaining_to_reach_20m_faucet_balance_tcreg: proposal.remaining_to_reach_20m_faucet_balance_tcreg,
  output,
}, null, 2))
