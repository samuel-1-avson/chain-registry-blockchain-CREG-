import React from 'react'

const PHASE = import.meta.env.VITE_TESTNET_PHASE || ''
const SINGLE_VALIDATOR = import.meta.env.VITE_SINGLE_VALIDATOR === 'true'

/**
 * Shown on alpha / single-validator testnets so users do not expect fleet-wide verified status.
 */
export function TestnetPhaseBanner() {
  if (!PHASE && !SINGLE_VALIDATOR) return null

  const lines = []
  if (PHASE) {
    lines.push(`Testnet phase: ${PHASE}`)
  }
  if (SINGLE_VALIDATOR) {
    lines.push(
      'Single-validator alpha — packages may stay pending until NET-301 multi-validator quorum ships.'
    )
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
