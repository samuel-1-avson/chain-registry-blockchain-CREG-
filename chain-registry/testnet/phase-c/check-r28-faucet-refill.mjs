// Read-only refill decision from the actual public faucet and Sepolia ledger.
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'

const base = 'https://faucet.testnet.cregnet.dev'
const unit = 10n ** 18n
async function readJson(route) {
  const response = await fetch(base + route, { signal: AbortSignal.timeout(20000) })
  assert.equal(response.status, 200, `${route} returned HTTP ${response.status}`)
  return response.json()
}

const [stats, supply] = await Promise.all([readJson('/api/stats'), readJson('/api/supply')])
assert.equal(stats.status, 'available')
assert.equal(supply.status, 'available')
assert.equal(supply.chain_id, 11155111)
assert.equal(stats.faucet_address.toLowerCase(), supply.faucet.address.toLowerCase())
assert.equal(stats.token_contract.toLowerCase(), supply.token_contract.toLowerCase())
assert.equal(stats.token_hot_wallet_float_wei, (100_000n * unit).toString())
assert.equal(stats.token_allocation_wei, (1_000_000n * unit).toString())
const balance = BigInt(stats.faucet_balance)
const float = BigInt(stats.token_hot_wallet_float_wei)
const trigger = 25_000n * unit
if (balance > float) throw new Error('Faucet balance exceeds its approved working limit')
const due = balance <= trigger
const amount = due ? float - balance : 0n
const report = {
  observed_at_utc: new Date().toISOString(),
  network: 'Sepolia',
  token_contract: stats.token_contract,
  faucet_address: stats.faucet_address,
  faucet_balance_wei: balance.toString(),
  refill_trigger_wei: trigger.toString(),
  working_balance_limit_wei: float.toString(),
  refill_due: due,
  proposed_top_up_wei: amount.toString(),
  token_claims_enabled: stats.token_claims_enabled,
  ledger_reconciled: stats.ledger_reconciled,
  read_only: true,
  on_chain_transaction_sent: false,
}
const output = process.argv[2]
if (output) {
  await fs.mkdir(path.dirname(output), { recursive: true })
  await fs.writeFile(output, JSON.stringify(report, null, 2) + '\n')
}
console.log(JSON.stringify(report, null, 2))
