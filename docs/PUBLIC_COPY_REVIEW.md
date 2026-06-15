# Public copy review (MAIN-007 / L2 gate)

**Reviewed:** 2026-06-15  
**Scope:** Hub web, public quickstart, whitepaper, operator docs visible to external users  
**Verdict:** Pass for in-repo surfaces; social copy remains operator-owned outside this repo.

## Checked surfaces

| Surface | Status | Notes |
|---------|--------|-------|
| `hub-web` AlphaDisclaimer | Pass | "Public alpha — not mainnet or production" |
| `hub-web` HomePage / NetworkPage / FaqPage | Pass | Alpha framing; no mainnet-ready claims |
| `docs/PUBLIC_TESTNET_QUICKSTART.md` | Pass | SEC-401 + alpha limitations section |
| `docs/TESTNET_PHASE_SCOPE.md` | Pass | Defines verified/pending honestly |
| `docs/L2_PUBLIC_ALPHA_GATE_STATUS.md` | Pass | Tracks partial gates explicitly |
| `docs/WHITEPAPER.md` | Pass | Public alpha framing, SEC-401 open, no package-safety guarantee |

## Out of repo (manual before wide marketing)

| Surface | Action |
|---------|--------|
| Social posts (X, LinkedIn, etc.) | Use hub FAQ language: coordinated testnet, waitlist, not mainnet |
| Waitlist landing copy | Firebase deploy — confirm matches hub disclaimers |

## Banned phrases (until SEC-401 + L3)

- "Production-ready" / "mainnet-ready" / "enterprise-grade security"
- "Audited" without naming vendor + report date
- "Guaranteed" package safety (use "verified after validator quorum")

## Sign-off

In-repo public copy is **alpha-safe** for waitlist + testnet onboarding. Widen marketing only after SEC-401 vendor is booked.
