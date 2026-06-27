# Next work — prioritized backlog

**Updated:** 2026-06-26  
**Planning hub:** [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md)  
**Context:** [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md), [WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md)

---

## Current phase

- **Protocol hardening:** Iteration 6 complete (BatchOperations + finalize relays; forge 37/37).
- **Hosting:** Waitlist live on Firebase; **public testnet GCP stack may be stopped** to save cost — see [WAITLIST-ONLY-HOSTING.md](./WAITLIST-ONLY-HOSTING.md).
- **Next decision:** Resume testnet for validator onboarding **or** continue product-only phase (waitlist + Contribution Arena).

---

## P0 — Gating work

| # | ID | Status | Task | Acceptance |
|---|-----|--------|------|------------|
| 1 | **SEC-401** | open | External security audit | Scope: [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md). Evidence pack from Phase 0 remediation. Vendor + start date TBD. |
| 2 | **AUDIT-PACK** | open | Audit evidence pack | DEEP_DIVE status rollup, forge/cargo summaries, threat model — [WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md) Phase 0 |
| 3 | **TESTNET-RESUME** | ready | Resume public fleet when validators ready | `testnet/gcp/resume-testnet-hosting.ps1` + `hosting-301-verify.ps1`; [testnet/gcp/README.md](../chain-registry/testnet/gcp/README.md) |

---

## P1 — Quality (next 2–4 weeks)

| # | ID | Status | Task | Acceptance |
|---|-----|--------|------|------------|
| 4 | **ISSUE-008** | **done** | Finalize relay enforcement | Default `enforceFinalizeRelays=true`; forge tests; [contracts/README.md](../chain-registry/contracts/README.md) |
| 5 | **ISSUE-003** | **done** | BatchOperations publisher identity | `submitPackageFor` + forge tests |
| 6 | **GOSSIP-301** | **done** | Gossip/PBFT validation (Iter 5) | Metrics + unit tests |
| 7 | **BRIDGE-301** | open | L1 bridge soak | `CREG_BRIDGE_KEY` on validator; batch observed or documented skip + alert |
| 8 | **SOAK-301** | open | Incentive / load soak | [testnet/SOAK_TEST.md](../chain-registry/testnet/SOAK_TEST.md); Phase 4 remediation |
| 9 | **SEC-402** | open | Network partition chaos | `k8s/55-network-partition-test.yaml` + postmortem |
| 10 | **ARENA-HUB** | open | Contribution Arena → Hub API | Apply `002_contribution_arena.sql`; [HUB_INTEGRATION.md](../CREG-Contribution-Arena/docs/HUB_INTEGRATION.md) |

---

## P2 — Soon (not blocking)

| # | ID | Task |
|---|-----|------|
| 11 | **REM-211+** | Grafana dashboards per [observability/README.md](../chain-registry/observability/README.md) |
| 12 | **VAL-SCALE** | Genesis Alpha validator cohort (10k slots, batched) — Phase 3 |
| 13 | **ZK-CIRCUIT** | Circuit regen + trusted setup if binding changes — ISSUE-001 follow-up |
| 14 | **SHIM-301** | Shim security vs `creg install` — DEEP_DIVE shim notes |

---

## Completed (reference)

| ID | Done | Notes |
|----|------|-------|
| NET-301 | 2026-06-09 | 2-validator PBFT quorum |
| SANDBOX-301 | 2026-06-10 | Real nsjail sandbox |
| DIST-301 | 2026-06-10 | `v0.1.0-testnet` / `v0.1.2-testnet` binaries |
| HOSTING-301 | 2026-06-10 | `testnet.cregnet.dev` (resumable) |
| DOC-301 | 2026-06-08 | Doc index rationalization |
| GAPS-Iter5 | 2026-06-26 | Gossip/PBFT hardening |
| GAPS-Iter6 | 2026-06-26 | Batch + finalize relay forge tests |

---

## Quick commands

```powershell
cd chain-registry

# Local verification
cargo test --workspace
cd contracts; forge test

# Maintainer seed fleet env
.\testnet\init-sepolia-fleet-env.ps1

# Resume public hosting (when ready)
.\testnet\gcp\resume-testnet-hosting.ps1
.\testnet\hosting-301-verify.ps1 -BaseDomain testnet.cregnet.dev

# Stop public hosting (cost saving)
.\testnet\gcp\stop-testnet-hosting.ps1
```

---

_Update this file when an item ships; mirror in [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md)._