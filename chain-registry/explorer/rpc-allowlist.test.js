// node --test explorer/rpc-allowlist.test.js — ADR-019 Increment 1 (ECA-38).
import test from 'node:test'
import assert from 'node:assert/strict'
import rpc from './rpc-allowlist.js'

const call = (method, extra = {}) => JSON.stringify({ jsonrpc: '2.0', id: 1, method, params: [], ...extra })

test('allowlisted single calls pass', () => {
  for (const method of rpc.ALLOWED_METHODS) {
    assert.deepEqual(rpc.check(call(method)), { ok: true }, method)
  }
})

test('expensive and administrative methods are refused', () => {
  for (const method of [
    'eth_getLogs',
    'debug_traceTransaction',
    'trace_block',
    'txpool_content',
    'admin_peers',
    'personal_sign',
    'eth_sign',
    'eth_sendTransaction',
    'eth_newFilter',
  ]) {
    const verdict = rpc.check(call(method))
    assert.equal(verdict.ok, false, method)
    assert.equal(verdict.status, 403, method)
  }
})

test('one bad method poisons a batch', () => {
  const batch = JSON.stringify([
    { jsonrpc: '2.0', id: 1, method: 'eth_chainId' },
    { jsonrpc: '2.0', id: 2, method: 'eth_getLogs' },
  ])
  assert.equal(rpc.check(batch).status, 403)
})

test('malformed and oversized batches are refused', () => {
  assert.equal(rpc.check('not json').status, 400)
  assert.equal(rpc.check('[]').status, 400)
  assert.equal(rpc.check('null').status, 400)
  assert.equal(rpc.check('[[]]').status, 400)
  assert.equal(rpc.check(JSON.stringify({ id: 1 })).status, 400)
  assert.equal(rpc.check(call('x'.repeat(65))).status, 400)
  const big = JSON.stringify(Array.from({ length: rpc.MAX_BATCH + 1 }, (_, i) => ({ id: i, method: 'eth_chainId' })))
  assert.equal(rpc.check(big).status, 400)
})

test('refusal messages never echo control or markup characters', () => {
  const verdict = rpc.check(call('evil<script>\u001b]0;x'))
  assert.equal(verdict.status, 403)
  assert.match(verdict.message, /^method not allowed: [A-Za-z0-9_?]+$/)
})
