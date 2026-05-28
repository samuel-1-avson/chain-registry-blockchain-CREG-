# Chain Registry — Documentation Index

Central documentation for the Chain Registry monorepo (`chain-registry/`).

## Analysis & planning

| Document | Description |
|----------|-------------|
| [SYSTEM_FULL_ANALYSIS_REPORT.md](./SYSTEM_FULL_ANALYSIS_REPORT.md) | Architecture, APIs, data stores, blockchain, wallets, readiness scores |
| [SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md](./SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md) | Security remediation backlog and phased execution plan |

## Security & operations

| Document | Description |
|----------|-------------|
| [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) | Hot keys, unsafe env vars, TLS, operator API keys |
| [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md) | Live status of SEC-/REM-/DOC- work items |

## Data & API

| Document | Description |
|----------|-------------|
| [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) | Sepolia contract deploy, chain-spec sign/publish, node env |
| [PHASE2_SEPOLIA_KICKOFF.md](./PHASE2_SEPOLIA_KICKOFF.md) | Phase 2 Option A — ordered Sepolia-first checklist |
| [PHASE2_CLOSEOUT.md](./PHASE2_CLOSEOUT.md) | Phase 2 ship checklist, Etherscan links, deferred items |
| [DATABASE_SCHEMA.md](./DATABASE_SCHEMA.md) | Canonical PostgreSQL schema (`db-sync`) vs legacy testnet SQL |
| [API_COOKBOOK.md](./API_COOKBOOK.md) | REST examples by route group |

## Related paths in the repo

- Application code: `chain-registry/`
- Root README: [../README.md](../README.md)
- Migrations: `chain-registry/migrations/`
