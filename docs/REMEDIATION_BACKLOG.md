# Remediation Backlog

Track security and remediation work from [SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md](./SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md).

| ID | Status | Phase | Notes |
|----|--------|-------|-------|
| DOC-101 | done | 1 | Docs index + root README links |
| DOC-102 | done | 1 | API cookbook |
| DOC-103 | done | 1 | Security ops runbook |
| DOC-104 | done | 1 | Backlog sync |
| REM-101 | done | 1 | Explorer `relayer.js` → `/sponsor`, `/status/:id`, `/policy`, `/quote` |
| REM-102 | done | 1 | `migrations/001_db_sync_bootstrap.sql`, `002_testnet_extras.sql` |
| SEC-104 | done | 1 | Rate limits on `/v1/publisher/packages`, `/v1/validator/consensus/vote` |
| SEC-201 | done | 1 | ZKVerifier — 6/6 tests pass (`forge test --match-contract ZKVerifier`) |
| SEC-102 | done | 1 | `validate_production_security()` fail-fast at node boot |
| SEC-106 | done | 1 | `creg doctor` PBFT + production safety checks |
| REM-201 | done | 2 | Governance HTTP 501 + explorer hidden unless `VITE_GOVERNANCE_ENABLED=true` |
| REM-103 | done | 2 | JSON cursor sidecar + idempotency/reorg tests; atomic save |
| REM-210 | done | 2 | [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) |
| SEC-203 | done | 2 | `creg chain-spec validate` — genesis hash + optional `.sig` Ed25519 verify |
| SEC-101 | done | 2 | Hot-key runbook rotation + `.env.example` placeholders |
| SEC-101b | done | 2 | Startup WARN with fingerprint (bridge, faucet, relayer) when not testnet |
| SEC-302 | pending | 3 | CrossChainRegistry |

_Update status when PRs merge._
