import React, { useMemo, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { nodeApi } from '../api/node.js'
import { useFetch } from '../hooks/useFetch.js'
import { Hash } from '../components/Hash.jsx'
import { SkeletonCard } from '../components/Skeleton.jsx'
import { ErrorState, EmptyState } from '../components/ErrorState.jsx'
import { StatusBadge } from '../components/StatusBadge.jsx'
import { ShareButton } from '../components/ShareButton.jsx'

/**
 * /proof — Light-client Merkle proof verifier.
 *
 * Allows users to:
 * 1. Enter a package canonical (or paste a proof JSON).
 * 2. Fetch the proof from /v1/public/packages/:canonical/proof.
 * 3. Verify the inclusion proof client-side against the chain's Merkle root.
 * 4. Display the proof path with visual node indicators.
 */
export default function ProofVerifier() {
  const [canonical, setCanonical] = useState('')
  const [pastedProof, setPastedProof] = useState(null)
  const [fetchTriggered, setFetchTriggered] = useState(false)
  const [verifyResult, setVerifyResult] = useState(null)

  const proof = useFetch(
    (signal) => nodeApi.packageProof(canonical, signal),
    { deps: [canonical], enabled: fetchTriggered && !!canonical.trim() },
  )

  const activeProof = pastedProof || proof.data
  const normalizedProof = useMemo(() => normalizeProofResponse(activeProof), [activeProof])

  const handleFetch = (e) => {
    e.preventDefault()
    if (!canonical.trim()) return
    setPastedProof(null)
    setVerifyResult(null)
    setFetchTriggered(true)
  }

  const handlePaste = (e) => {
    try {
      const txt = e.target.value.trim()
      if (!txt) { setPastedProof(null); return }
      const parsed = JSON.parse(txt)
      setPastedProof(parsed)
      setVerifyResult(null)
      if (parsed.canonical) setCanonical(parsed.canonical)
    } catch {
      // ignore invalid JSON
    }
  }

  const handleVerify = async () => {
    if (!normalizedProof) return
    const result = await verifyMerkleProof(normalizedProof)
    setVerifyResult(result)
  }

  return (
    <div style={{ display: 'grid', gap: 'var(--space-6)' }}>
      <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
        <div>
          <h1 style={{ margin: 0, fontSize: 20 }}>Proof verifier</h1>
          <p style={{ color: 'var(--text-tertiary)', fontSize: 12, marginTop: 4 }}>
            Verify Merkle inclusion proofs for on-chain packages — light-client compatible.
          </p>
        </div>
        <ShareButton />
      </header>

      {/* Fetch form */}
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <h2 style={{ margin: 0, fontSize: 14 }}>Fetch proof by package</h2>
        <form onSubmit={handleFetch} style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <input
            type="text"
            value={canonical}
            onChange={(e) => { setCanonical(e.target.value); setFetchTriggered(false) }}
            placeholder="Package canonical (e.g. npm/express@4.18.0)"
            style={{
              flex: 1, minWidth: 240, padding: '8px 12px',
              background: 'var(--bg-elevated)', color: 'var(--text-primary)',
              border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
              fontSize: 13, fontFamily: 'var(--font-mono)',
            }}
          />
          <button type="submit" disabled={!canonical.trim() || proof.loading} style={btnStyle}>
            {proof.loading ? '⏳ Fetching…' : '⇩ Fetch proof'}
          </button>
        </form>
      </section>

      {/* Paste section */}
      <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-3)' }}>
        <h2 style={{ margin: 0, fontSize: 14 }}>Or paste proof JSON</h2>
        <textarea
          rows={5}
          onChange={handlePaste}
          placeholder='Paste a proof JSON here: { "block_hash": "…", "block_header": { ... }, "proof": { "tx_hash": "…", "expected_root": "…", "path": [ { "sibling_hash": "…", "is_right": true } ] } }'
          style={{
            width: '100%', padding: '8px 12px', resize: 'vertical',
            background: 'var(--bg-elevated)', color: 'var(--text-primary)',
            border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)',
            fontSize: 11, fontFamily: 'var(--font-mono)',
          }}
        />
      </section>

      {/* Error state */}
      {proof.error && !pastedProof && (
        <div className="ce-card" style={{ borderColor: 'var(--accent-error)' }}>
          <p style={{ color: 'var(--accent-error)', fontSize: 12, margin: 0 }}>
            Could not fetch proof: {proof.error.message || 'Not found'}
          </p>
        </div>
      )}

      {/* Proof display */}
      {activeProof && normalizedProof && (
        <section className="ce-card" style={{ display: 'grid', gap: 'var(--space-4)' }}>
          <header style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <h2 style={{ margin: 0, fontSize: 15 }}>Proof data</h2>
            <button type="button" onClick={handleVerify} style={btnAccentStyle}>
              ✓ Verify proof
            </button>
          </header>

          {/* Proof fields */}
          <div style={{ display: 'grid', gap: 'var(--space-2)' }}>
            {normalizedProof.merkleRoot && <Row k="Merkle root" v={<Hash value={normalizedProof.merkleRoot} full showCopy />} />}
            {normalizedProof.leafHash && <Row k="Leaf hash" v={<Hash value={normalizedProof.leafHash} full showCopy />} />}
            {normalizedProof.blockHash && <Row k="Block hash" v={<Hash value={normalizedProof.blockHash} kind="block-hash" full showCopy />} />}
            {normalizedProof.canonical && <Row k="Canonical" v={normalizedProof.canonical} />}
            {normalizedProof.blockHeight != null && (
              <Row k="Block" v={
                <Link to={`/block/${normalizedProof.blockHeight}`} style={{ color: 'var(--accent-primary-light)' }}>
                  #{normalizedProof.blockHeight}
                </Link>
              } />
            )}
          </div>

          {/* Proof path visualization */}
          {normalizedProof.path.length > 0 && (
            <div>
              <div style={{ fontSize: 11, color: 'var(--text-tertiary)', textTransform: 'uppercase', letterSpacing: '0.04em', marginBottom: 8 }}>
                Proof path ({normalizedProof.path.length} nodes)
              </div>
              <div style={{ display: 'grid', gap: 6 }}>
                {normalizedProof.path.map((node, i) => {
                  const dir = node.is_right ? 'right' : 'left'
                  return (
                    <div key={i} style={{
                      display: 'flex', alignItems: 'center', gap: 8,
                      padding: '6px 10px', background: 'var(--bg-elevated)',
                      borderRadius: 'var(--radius-sm)',
                      borderLeft: `3px solid ${i === 0 ? 'var(--accent-success)' : i === normalizedProof.path.length - 1 ? 'var(--accent-primary-light)' : 'var(--border)'}`,
                    }}>
                      <span style={{ fontSize: 10, fontFamily: 'var(--font-mono)', color: 'var(--text-tertiary)', minWidth: 24 }}>{i}</span>
                      {dir && <StatusBadge variant="muted">{dir}</StatusBadge>}
                      <Hash value={node.sibling_hash} start={12} end={10} showCopy />
                      {i === 0 && <span style={{ fontSize: 9, color: 'var(--accent-success)' }}>leaf</span>}
                      {i === normalizedProof.path.length - 1 && <span style={{ fontSize: 9, color: 'var(--accent-primary-light)' }}>root</span>}
                    </div>
                  )
                })}
              </div>
            </div>
          )}

          {/* Verification result */}
          {verifyResult != null && (
            <div style={{
              padding: 'var(--space-3) var(--space-4)',
              borderRadius: 'var(--radius-sm)',
              background: verifyResult.valid ? 'rgba(34,197,94,0.08)' : 'rgba(239,68,68,0.08)',
              border: `1px solid ${verifyResult.valid ? 'var(--accent-success)' : 'var(--accent-error)'}`,
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ fontSize: 18 }}>{verifyResult.valid ? '✅' : '❌'}</span>
                <div>
                  <div style={{ fontWeight: 700, color: verifyResult.valid ? 'var(--accent-success)' : 'var(--accent-error)' }}>
                    {verifyResult.valid ? 'Proof is valid' : 'Proof verification failed'}
                  </div>
                  <div style={{ fontSize: 11, color: 'var(--text-secondary)', marginTop: 2 }}>
                    {verifyResult.message}
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Download */}
          <button
            type="button"
            onClick={() => {
              const blob = new Blob([JSON.stringify(activeProof, null, 2)], { type: 'application/json' })
              const url = URL.createObjectURL(blob)
              const a = document.createElement('a')
              a.href = url
              a.download = `proof-${(activeProof.canonical || canonical || 'unknown').replace(/[^a-zA-Z0-9@._-]/g, '_')}.json`
              a.click()
              URL.revokeObjectURL(url)
            }}
            style={btnStyle}
          >
            ⇩ Download proof JSON
          </button>
        </section>
      )}

      {/* Help section */}
      {!activeProof && !proof.loading && (
        <section className="ce-card">
          <h2 style={{ margin: '0 0 var(--space-3) 0', fontSize: 14 }}>How it works</h2>
          <div style={{ display: 'grid', gap: 8, fontSize: 12, color: 'var(--text-secondary)', lineHeight: 1.6 }}>
            <p style={{ margin: 0 }}>
              Every package published to the Chain Registry is committed into a Merkle tree.
              The root hash of this tree is anchored to L1 via the bridge, creating an immutable proof of inclusion.
            </p>
            <ol style={{ margin: 0, paddingLeft: 20, display: 'grid', gap: 4 }}>
              <li>Enter a package canonical (e.g., <code>npm/express@4.18.0</code>) above</li>
              <li>The verifier fetches the Merkle proof from the node</li>
              <li>Recompute hashes along the proof path using each <code>proof.path[].is_right</code> direction bit</li>
              <li>Compare the recomputed root against <code>proof.expected_root</code> and <code>block_header.merkle_root</code></li>
            </ol>
            <p style={{ margin: 0, color: 'var(--text-tertiary)', fontSize: 11 }}>
              This verification can be performed by any light client without downloading the full chain.
            </p>
          </div>
        </section>
      )}
    </div>
  )
}

/**
 * Normalize the node's LightClientResponse into a UI-friendly proof shape.
 */
function normalizeProofResponse(proofResponse) {
  if (!proofResponse) return null

  if (proofResponse.proof && proofResponse.block_header) {
    return {
      canonical: proofResponse.canonical,
      blockHash: proofResponse.block_hash,
      blockHeight: proofResponse.block_header.height,
      blockMerkleRoot: proofResponse.block_header.merkle_root,
      merkleRoot: proofResponse.proof.expected_root,
      leafHash: proofResponse.proof.tx_hash,
      path: Array.isArray(proofResponse.proof.path) ? proofResponse.proof.path : [],
    }
  }

  return {
    canonical: proofResponse.canonical,
    blockHash: proofResponse.block_hash,
    blockHeight: proofResponse.block_height,
    blockMerkleRoot: proofResponse.block_merkle_root,
    merkleRoot: proofResponse.root || proofResponse.merkle_root || proofResponse.expected_root,
    leafHash: proofResponse.leaf || proofResponse.tx_hash,
    path: Array.isArray(proofResponse.path)
      ? proofResponse.path.map((step) => {
          if (typeof step === 'string') return { sibling_hash: step, is_right: null }
          return {
            sibling_hash: step.sibling_hash || step.hash,
            is_right: typeof step.is_right === 'boolean'
              ? step.is_right
              : step.direction === 'right'
                ? true
                : step.direction === 'left'
                  ? false
                  : null,
          }
        })
      : [],
  }
}

async function sha256Hex(input) {
  const bytes = new TextEncoder().encode(input)
  const digest = await crypto.subtle.digest('SHA-256', bytes)
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('')
}

async function verifyMerkleProof(proof) {
  if (!proof) return { valid: false, message: 'No proof data' }
  if (!proof.merkleRoot) return { valid: false, message: 'Proof missing expected Merkle root' }
  if (!proof.path || proof.path.length === 0) return { valid: false, message: 'Proof missing path' }
  if (!proof.leafHash) return { valid: false, message: 'Proof missing leaf hash' }

  const pathValid = proof.path.every((node) => {
    return typeof node?.sibling_hash === 'string' && node.sibling_hash.length >= 32
  })

  if (!pathValid) return { valid: false, message: 'One or more path nodes have invalid hash format' }

  const directionValid = proof.path.every((node) => typeof node?.is_right === 'boolean')
  if (!directionValid) {
    return { valid: false, message: 'Proof path is missing LightClientResponse direction bits (proof.path[].is_right)' }
  }

  if (proof.blockMerkleRoot && proof.blockMerkleRoot !== proof.merkleRoot) {
    return { valid: false, message: 'Proof root does not match the block header Merkle root' }
  }

  let current = proof.leafHash
  for (const step of proof.path) {
    current = step.is_right
      ? await sha256Hex(`${current}${step.sibling_hash}`)
      : await sha256Hex(`${step.sibling_hash}${current}`)
  }

  if (current !== proof.merkleRoot) {
    return {
      valid: false,
      message: `Recomputed root ${current.slice(0, 16)}… did not match expected root ${proof.merkleRoot.slice(0, 16)}…`,
    }
  }

  return {
    valid: true,
    message: `Merkle proof verified across ${proof.path.length} path nodes. Root: ${proof.merkleRoot.slice(0, 16)}…`
      + (proof.blockHeight != null ? ` · anchored at block #${proof.blockHeight}` : ''),
  }
}

function Row({ k, v }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '140px 1fr', gap: 'var(--space-3)', alignItems: 'center' }}>
      <span style={{ color: 'var(--text-tertiary)', fontSize: 12, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{k}</span>
      <span style={{ color: 'var(--text-primary)', fontSize: 13, wordBreak: 'break-all' }}>{v ?? '—'}</span>
    </div>
  )
}

const btnStyle = {
  padding: '8px 16px',
  background: 'var(--surface)',
  border: '1px solid var(--border)',
  borderRadius: 'var(--radius-sm)',
  color: 'var(--text-secondary)',
  fontSize: 12, fontWeight: 600, cursor: 'pointer',
  transition: 'all var(--transition-fast)',
}

const btnAccentStyle = {
  ...btnStyle,
  border: '1px solid var(--accent-primary)',
  color: 'var(--accent-primary-light)',
  background: 'rgba(232, 163, 92, 0.08)',
}
