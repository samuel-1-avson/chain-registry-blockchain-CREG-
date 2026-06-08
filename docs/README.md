# Chain Registry — Documentation Index

Central documentation for the Chain Registry monorepo (`chain-registry/`).

## Analysis & planning

| Document | Description |
|----------|-------------|
| [SYSTEM_FULL_ANALYSIS_REPORT.md](./SYSTEM_FULL_ANALYSIS_REPORT.md) | Architecture, APIs, data stores, blockchain, wallets, readiness scores |
| [SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md](./SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md) | Security remediation backlog and phased execution plan |
| [NEXT_WORK.md](./NEXT_WORK.md) | Prioritized testnet readiness checklist (owners + acceptance criteria) |
| [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) | External testnet phase scope — observer vs validator, limits, NET-301 deferral |
| [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | One-page publisher / developer / validator quickstart for Sepolia testnet |
| [session_analysis_report.md](./session_analysis_report.md) | AI session postmortem — root causes, hotspots, severity |
| [prompt_improvement_tips.md](./prompt_improvement_tips.md) | Copy-paste prompts for bounded work on this repo |

## Security & operations

| Document | Description |
|----------|-------------|
| [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) | Hot keys, unsafe env vars, TLS, operator API keys |
| [WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md) | Ed25519 vs Ethereum wallet keys (SEC-105) |
| [SEPOLIA_SECOND_OPERATOR_CHECKLIST.md](./SEPOLIA_SECOND_OPERATOR_CHECKLIST.md) | OPS-201 sign-off for Option A reuse (closed 2026-05-30) |
| [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) | What external testnet participants should expect |
| [OBSERVABILITY_SEPOLIA.md](./OBSERVABILITY_SEPOLIA.md) | Prometheus scrape for local Sepolia node (REM-211) |
| [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md) | Live status of SEC-/REM-/DOC- work items |

## Data & API

| Document | Description |
|----------|-------------|
| [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) | Sepolia contract deploy, chain-spec sign/publish, node env |
| [PHASE2_SEPOLIA_KICKOFF.md](./PHASE2_SEPOLIA_KICKOFF.md) | Phase 2 Option A — ordered Sepolia-first checklist |
| [PHASE2_CLOSEOUT.md](./PHASE2_CLOSEOUT.md) | Phase 2 ship checklist, Etherscan links, deferred items |
| [PHASE3_KICKOFF.md](./PHASE3_KICKOFF.md) | Phase 3 start — D4 cross-chain decision (SEC-303c), epic order |
| [adr/ADR-KMS-HOT-KEYS.md](./adr/ADR-KMS-HOT-KEYS.md) | SEC-301a — KMS/Vault vs env for bridge, faucet, relayer hot keys |
| [DATABASE_SCHEMA.md](./DATABASE_SCHEMA.md) | Canonical PostgreSQL schema (`db-sync`) vs legacy testnet SQL |
| [API_COOKBOOK.md](./API_COOKBOOK.md) | REST examples by route group |

## Related paths in the repo

- Application code: `chain-registry/`
- Root README: [../README.md](../README.md)
- Migrations: `chain-registry/migrations/`
