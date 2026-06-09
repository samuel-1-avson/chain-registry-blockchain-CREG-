import React from 'react'

const PHASE = import.meta.env.VITE_TESTNET_PHASE || ''
const SINGLE_VALIDATOR = import.meta.env.VITE_SINGLE_VALIDATOR === 'true'
const DEV_SANDBOX_CAVEAT = import.meta.env.VITE_DEV_SANDBOX_CAVEAT === 'true'

/**
 * Shown on alpha / lab testnets so users know quorum and sandbox limitations.
 */
export function TestnetPhaseBanner() {
  if (!PHASE && !SINGLE_VALIDATOR && !DEV_SANDBOX_CAVEAT) return null

  const lines = []
  if (PHASE) {
    lines.push(`Testnet phase: ${PHASE}`)
  }
  if (SINGLE_VALIDATOR) {
    lines.push(
      'Single-validator alpha — packages may stay pending until NET-301 multi-validator quorum ships.'
    )
  } else if (PHASE === 'coordinated-lab' || DEV_SANDBOX_CAVEAT) {
    lines.push(
      'NET-301: 2-validator quorum proven in maintainer lab. Behavioural sandbox may use CREG_DEV_SANDBOX on Windows — not production-grade until SANDBOX-301.'
    )
  }
  if (DEV_SANDBOX_CAVEAT && !SINGLE_VALIDATOR) {
    lines.push('No public hosted node URL in chain spec yet — set CREG_NODE_URL to your operator endpoint.')
  }

  return (
    <div
      role="status"
      style={{
        background: 'rgba(245, 158, 11, 0.12)',
        borderBottom: '1px solid rgba(245, 158, 11, 0.35)',
        color: 'var(--accent-warning, #f59e0b)',
        fontSize: 12,
        fontWeight: 600,
        padding: '8px 24px',
        textAlign: 'center',
      }}
    >
      {lines.join(' · ')}
    </div>
  )
}
