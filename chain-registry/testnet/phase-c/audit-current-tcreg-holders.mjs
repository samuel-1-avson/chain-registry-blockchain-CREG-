// Read-only, single-block audit of the actual R7 token on Sepolia.
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'

const manifest = JSON.parse(await fs.readFile(new URL('./deployment-manifest.json', import.meta.url)))
const rpcUrl = 'https://api.testnet.cregnet.dev/sepolia'
const token = manifest.cregToken
const unit = 10n ** 18n
const holders = {
  treasury: manifest.deployer,
  faucet: '0x721C8dBD8208CA9dBdcC12AE9339225458608B81',
  governance: manifest.governance,
  staking: manifest.staking,
  validator_1: '0x91e4dfc18bf015051676f00f23782d0770b95a7a',
  recipient_2: '0x5f424b4ec88073fd461eb194833681a31adfa311',
  validator_rewards: manifest.validatorRewards,
  pinning_rewards: manifest.pinningRewards,
}

let nextId = 1
async function rpc(method, params) {
  const response = await fetch(rpcUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: nextId++, method, params }),
    signal: AbortSignal.timeout(20000),
  })
  assert.equal(response.status, 200, `${method} HTTP status`)
  const body = await response.json()
  if (body.error) throw new Error(`${method} failed: ${body.error.message}`)
  assert.equal(typeof body.result, 'string', `${method} returned no scalar result`)
  return body.result
}

function formatTokens(raw) {
  const whole = raw / unit
  const fraction = (raw % unit).toString().padStart(18, '0').replace(/0+$/, '')
  return fraction ? `${whole}.${fraction}` : whole.toString()
}

assert.equal(BigInt(await rpc('eth_chainId', [])), 11155111n)
const blockTag = await rpc('eth_blockNumber', [])
const totalSupply = BigInt(await rpc('eth_call', [{ to: token, data: '0x18160ddd' }, blockTag]))
const balances = {}
for (const [name, address] of Object.entries(holders)) {
  const data = '0x70a08231' + address.slice(2).toLowerCase().padStart(64, '0')
  balances[name] = BigInt(await rpc('eth_call', [{ to: token, data }, blockTag]))
}
const accounted = Object.values(balances).reduce((sum, value) => sum + value, 0n)
assert.ok(accounted <= totalSupply, 'Known balances exceed total supply')
const report = {
  observed_at_utc: new Date().toISOString(),
  chain_id: 11155111,
  block_number: Number(BigInt(blockTag)),
  token_contract: token,
  total_supply_tcreg: formatTokens(totalSupply),
  accounted_supply_tcreg: formatTokens(accounted),
  unaccounted_supply_tcreg: formatTokens(totalSupply - accounted),
  holders: Object.fromEntries(Object.entries(holders).map(([name, address]) => [name, {
    address,
    balance_tcreg: formatTokens(balances[name]),
  }])),
  data_source: rpcUrl,
  requests_mocked: false,
  on_chain_transaction_sent: false,
}
const output = process.argv[2]
if (output) {
  await fs.mkdir(path.dirname(output), { recursive: true })
  await fs.writeFile(output, JSON.stringify(report, null, 2) + '\n')
}
console.log(JSON.stringify(report, null, 2))
assert.equal(accounted, totalSupply, 'Known holder balances do not account for all minted tokens')
