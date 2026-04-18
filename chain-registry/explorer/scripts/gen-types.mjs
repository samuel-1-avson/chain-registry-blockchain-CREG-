#!/usr/bin/env node
// Generate explorer TypeScript types from a running node's OpenAPI schema.
//
//   OPENAPI_URL=http://127.0.0.1:8080/v1/openapi.json npm run gen-types
//
// Resolution order for the source spec:
//   1. CLI arg:       node scripts/gen-types.mjs <url-or-file>
//   2. Env var:       OPENAPI_URL
//   3. Default URL:   http://127.0.0.1:8080/v1/openapi.json
//
// The script writes src/api/types.ts. Check the output into version control
// so the explorer compiles without a running node.

import fs from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import openapiTS, { astToString } from 'openapi-typescript'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const OUT_FILE = path.resolve(__dirname, '..', 'src', 'api', 'types.ts')

const source =
  process.argv[2] ||
  process.env.OPENAPI_URL ||
  'http://127.0.0.1:8080/v1/openapi.json'

async function loadSpec(src) {
  if (/^https?:\/\//i.test(src)) {
    console.error(`→ fetching ${src}`)
    const res = await fetch(src)
    if (!res.ok) throw new Error(`HTTP ${res.status} ${res.statusText}`)
    return res.text()
  }
  console.error(`→ reading ${src}`)
  return fs.readFile(src, 'utf8')
}

async function main() {
  const raw = await loadSpec(source)
  let spec
  try { spec = JSON.parse(raw) } catch { spec = raw }

  const ast = await openapiTS(spec, {
    alphabetize: true,
    defaultNonNullable: false,
  })
  const ts = astToString(ast)

  const banner = [
    '// AUTO-GENERATED — do not edit. Run `npm run gen-types` to refresh.',
    `// Source: ${source}`,
    `// Generated: ${new Date().toISOString()}`,
    '',
    '/* eslint-disable */',
    '',
  ].join('\n')

  await fs.writeFile(OUT_FILE, banner + ts, 'utf8')
  console.error(`✓ wrote ${path.relative(process.cwd(), OUT_FILE)} (${ts.length.toLocaleString()} chars)`)
}

main().catch((err) => {
  console.error('✗ gen-types failed:', err.message || err)
  process.exit(1)
})
