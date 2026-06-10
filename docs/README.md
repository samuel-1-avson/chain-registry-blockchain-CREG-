# Chain Registry — documentation index

Start at the [project README](../README.md), then use the sections below.

**Artifact map (scripts, compose, components):** [DELIVERABLES_INDEX.md](../chain-registry/DELIVERABLES_INDEX.md)

---

## Join the testnet

| Document | Audience |
|----------|----------|
| [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | Publishers, developers, validators |
| [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) | What to expect — limits, verified semantics, alpha scope |
| [WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md) | Ed25519 (packages) vs Ethereum EOA (staking) |

---

## Operators & hosting

| Document | Description |
|----------|-------------|
| [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) | Sepolia deploy, chain-spec sign/publish |
| [../chain-registry/testnet/OPERATOR.md](../chain-registry/testnet/OPERATOR.md) | 3-node Sepolia fleet (NET-301) |
| [../chain-registry/testnet/QUICKSTART.md](../chain-registry/testnet/QUICKSTART.md) | Fastest path to a local node |
| [../chain-registry/testnet/gcp-public-hosting.md](../chain-registry/testnet/gcp-public-hosting.md) | GCP VM + Cloudflare TLS (HOSTING-301) |
| [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md) | Cost model — VM + Firebase (two projects) |
| [WAITLIST_FIREBASE_DEPLOY.md](./WAITLIST_FIREBASE_DEPLOY.md) | Waitlist Firebase backend deploy |
| [../chain-registry/testnet/README.md](../chain-registry/testnet/README.md) | Testnet directory index |
| [../chain-registry/DOCKER.md](../chain-registry/DOCKER.md) | Docker compose profiles |

---

## Architecture & readiness

| Document | Description |
|----------|-------------|
| [../chain-registry/DEEP_DIVE_ANALYSIS.md](../chain-registry/DEEP_DIVE_ANALYSIS.md) | Architecture, data flows, ISSUE registry |
| [../chain-registry/TESTNET_READINESS_REPORT.md](../chain-registry/TESTNET_READINESS_REPORT.md) | Evidence-based readiness snapshot |
| [NEXT_WORK.md](./NEXT_WORK.md) | Prioritized open work (maintainers) |
| [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md) | SEC-/REM-/DOC- remediation status |

---

## Security & operations

| Document | Description |
|----------|-------------|
| [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) | Hot keys, TLS, rotation drills |
| [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md) | External audit RFP scope |
| [SEC-401-VENDOR-OUTREACH.md](./SEC-401-VENDOR-OUTREACH.md) | Vendor outreach template |
| [adr/ADR-KMS-HOT-KEYS.md](./adr/ADR-KMS-HOT-KEYS.md) | KMS vs env for operator keys |

Regenerate vendor emails: `chain-registry/testnet/prepare-sec-401-outreach.ps1` → `docs/archive/SEC-401-outreach-ready.md` (example output).

---

## Data & components

| Document | Description |
|----------|-------------|
| [DATABASE_SCHEMA.md](./DATABASE_SCHEMA.md) | PostgreSQL schema (`db-sync`) |
| [../chain-registry/migrations/README.md](../chain-registry/migrations/README.md) | DB migration order |
| [../chain-registry/contracts/README.md](../chain-registry/contracts/README.md) | Solidity contract status |
| [../chain-registry/observability/README.md](../chain-registry/observability/README.md) | Prometheus / Grafana |
| [../circuits/README.md](../circuits/README.md) | ZK Circom circuits |

---

## Archived (historical only)

[archive/README.md](./archive/README.md) — superseded plans and generated one-offs. Do not use for current operations.
