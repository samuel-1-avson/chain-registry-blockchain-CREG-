# Chain Registry — documentation index

> **Public visitors:** [PUBLIC.md](./PUBLIC.md) — protocol, validators, contracts only.  
> **Planning next work:** [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md) — testing, waitlist, Arena, testnet prep.

Start at the [project README](../README.md), then use the sections below.

---

## Planning & status

| Document | Purpose |
|----------|---------|
| [TESTNET_STATUS.md](./TESTNET_STATUS.md) | **Fleet live/paused matrix** — single source of truth |
| [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md) | **Start here (maintainers)** — four tracks: system test, product surfaces, validator onboarding, audit |
| [NEXT_WORK.md](./NEXT_WORK.md) | Prioritized IDs (NET-301, SEC-401, …) |
| [WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md) | Phased remediation + §6 living status |
| [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md) | SEC-/REM-/DOC- ticket status |
| [L2_PUBLIC_ALPHA_GATE_STATUS.md](./L2_PUBLIC_ALPHA_GATE_STATUS.md) | Public-alpha gate checklist |
| [CREG_SYSTEM_WORKFLOW.md](./CREG_SYSTEM_WORKFLOW.md) | Full system design + §15 iteration log |

---

## Join the testnet

| Document | Audience |
|----------|----------|
| [TESTNET_SCALE_ARCHITECTURE.md](./TESTNET_SCALE_ARCHITECTURE.md) | **10k+ validator topology** — start here for scale testnet |
| [GENESIS_ALPHA_COHORT_PLAYBOOK.md](./GENESIS_ALPHA_COHORT_PLAYBOOK.md) | Wave 1 cohort operators |
| [WHITEPAPER.md](./WHITEPAPER.md) | Developers, validators, auditors |
| [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | Publishers, developers, validators |
| [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) | Limits, verified semantics, alpha scope |
| [VALIDATOR_ONBOARDING_CHECKLIST.md](./VALIDATOR_ONBOARDING_CHECKLIST.md) | External validator operators |
| [WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md) | Ed25519 vs Ethereum EOA |

---

## Operators & hosting

| Document | Description |
|----------|-------------|
| [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) | Sepolia deploy, chain-spec sign/publish |
| [../chain-registry/testnet/OPERATOR.md](../chain-registry/testnet/OPERATOR.md) | Sepolia validator fleet |
| [../chain-registry/testnet/QUICKSTART.md](../chain-registry/testnet/QUICKSTART.md) | Fastest local node path |
| [../chain-registry/testnet/gcp/README.md](../chain-registry/testnet/gcp/README.md) | GCP scripts (resume/stop testnet, waitlist) |
| [WAITLIST-ONLY-HOSTING.md](./WAITLIST-ONLY-HOSTING.md) | Waitlist-only phase (testnet paused) |
| [GCP_PHASE2_RUNBOOK.md](./GCP_PHASE2_RUNBOOK.md) | Observer pool, Cloud Run hub-api, public API LB |
| [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md) | Cost model |
| [GCP-RPC-ARCHITECTURE.md](./GCP-RPC-ARCHITECTURE.md) | API/JSON-RPC on GCP |
| [GCP-VALIDATOR-FLEET.md](./GCP-VALIDATOR-FLEET.md) | Dedicated validator VM |
| [HYBRID-LOCAL-VALIDATORS.md](./HYBRID-LOCAL-VALIDATORS.md) | Legacy WireGuard path (deprecated) |
| [../chain-registry/testnet/README.md](../chain-registry/testnet/README.md) | Testnet directory index |
| [../chain-registry/DOCKER.md](../chain-registry/DOCKER.md) | Docker compose profiles |

---

## Product surfaces (non-node)

| Document | Description |
|----------|-------------|
| [WAITLIST_FIREBASE_DEPLOY.md](./WAITLIST_FIREBASE_DEPLOY.md) | Waitlist Firebase deploy |
| [WAITLIST-ADMIN-MONITORING.md](./WAITLIST-ADMIN-MONITORING.md) | Owner-only waitlist metrics |
| [../CREG-Contribution-Arena/README.md](../CREG-Contribution-Arena/README.md) | Contribution Arena (quests, findings) |
| [../CREG-Contribution-Arena/docs/HUB_INTEGRATION.md](../CREG-Contribution-Arena/docs/HUB_INTEGRATION.md) | Hub API migration boundary |
| [TESTNET-HUB-DESIGN.md](./TESTNET-HUB-DESIGN.md) | Hub API design |

---

## Architecture & readiness

| Document | Description |
|----------|-------------|
| [../chain-registry/DEEP_DIVE_ANALYSIS.md](../chain-registry/DEEP_DIVE_ANALYSIS.md) | Architecture + ISSUE registry |
| [../chain-registry/TESTNET_READINESS_REPORT.md](../chain-registry/TESTNET_READINESS_REPORT.md) | Historical readiness snapshot |
| [../chain-registry/contracts/README.md](../chain-registry/contracts/README.md) | Solidity contracts + relay model |
| [L1_L2_HARDENING.md](./L1_L2_HARDENING.md) | Bridge / ZK / cross-layer notes |

---

## Security & operations

| Document | Description |
|----------|-------------|
| [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) | Hot keys, TLS, rotation |
| [INCIDENT_RESPONSE_RUNBOOK.md](./INCIDENT_RESPONSE_RUNBOOK.md) | Protocol incidents |
| [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md) | External audit RFP scope |
| [SEC-401-VENDOR-OUTREACH.md](./SEC-401-VENDOR-OUTREACH.md) | Vendor email templates |
| [adr/ADR-KMS-HOT-KEYS.md](./adr/ADR-KMS-HOT-KEYS.md) | KMS vs env for operator keys |

Regenerate outreach copy: `chain-registry/testnet/prepare-sec-401-outreach.ps1` → [archive/SEC-401-outreach-ready.md](./archive/SEC-401-outreach-ready.md).

---

## Data & components

| Document | Description |
|----------|-------------|
| [DATABASE_SCHEMA.md](./DATABASE_SCHEMA.md) | PostgreSQL (`db-sync`) |
| [../chain-registry/migrations/README.md](../chain-registry/migrations/README.md) | DB migration order |
| [../chain-registry/observability/README.md](../chain-registry/observability/README.md) | Prometheus / Grafana |
| [../circuits/README.md](../circuits/README.md) | ZK Circom circuits |
| [../chain-registry/testnet/spec-server/README.md](../chain-registry/testnet/spec-server/README.md) | Signed chain-spec static host |

---

## Archived (historical only)

[archive/README.md](./archive/README.md) — superseded plans, one-off cohort exports, and deleted runbooks. **Do not use for current operations.**