import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { safeL1TxUrl, sanitizeUntrustedText, truncateGraphemes } from './untrustedText.js'

const ROOT_DIR = dirname(dirname(dirname(dirname(fileURLToPath(import.meta.url)))))

function walkFiles(dir) {
  const out = []
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) {
      out.push(...walkFiles(full))
    } else if (/\.(js|jsx|ts|tsx)$/.test(entry)) {
      out.push(full)
    }
  }
  return out
}

test('eco-adv-11: strips bidi/control bytes, preserves literal payload as inert text, and truncates by grapheme', () => {
  const payload = `<img src=x onerror=alert(1)>\u202E\u0007${'A'.repeat(4096)}`
  const sanitized = sanitizeUntrustedText(payload)
  assert.ok(!sanitized.includes('\u202E'), 'bidi RLO must be removed')
  assert.ok(!sanitized.includes('\u0007'), 'BEL control must be removed')
  assert.ok(sanitized.startsWith('<img src=x onerror=alert(1)>'))

  const truncated = truncateGraphemes(sanitized, 128)
  assert.equal(truncated.truncated, true)
  assert.ok(truncated.text.endsWith('...'))
  assert.ok(truncated.text.length <= 131)
})

test('safeL1TxUrl only permits 64-hex tx hashes on https origins', () => {
  const valid = `0x${'ab'.repeat(32)}`
  assert.equal(safeL1TxUrl(valid), `https://sepolia.etherscan.io/tx/${valid}`)
  assert.equal(safeL1TxUrl('javascript:alert(1)'), null)
  assert.equal(safeL1TxUrl('0x1234'), null)
  assert.equal(safeL1TxUrl(valid, 'http://insecure.example'), null)
})

test('ECO-INV-05 / ECA-41: explorer and hub-web contain zero dangerouslySetInnerHTML or innerHTML sinks', () => {
  const dirs = [
    join(ROOT_DIR, 'explorer', 'src'),
    join(ROOT_DIR, 'hub-web', 'src'),
  ]
  for (const dir of dirs) {
    for (const file of walkFiles(dir)) {
      if (file.endsWith('.test.js')) continue
      const content = readFileSync(file, 'utf8')
      assert.ok(
        !content.includes('dangerouslySetInnerHTML'),
        `Forbidden dangerouslySetInnerHTML found in ${file}`,
      )
      assert.ok(
        !/\.innerHTML\s*=/.test(content),
        `Forbidden .innerHTML assignment found in ${file}`,
      )
    }
  }
})
