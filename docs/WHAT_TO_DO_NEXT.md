# What to do next

**Updated:** 2026-08-23  
**HEAD:** `53c8089`  
**Audience:** Maintainers planning protocol work. Public GCP fleet is paused. Contribution Arena is a separate educational product and is not this workstream.

This is the **single planning entry point**. Shipped vs remaining vs testnet readiness: [PROTOCOL_PROGRESS.md](./PROTOCOL_PROGRESS.md). Prioritized IDs live in [NEXT_WORK.md](./NEXT_WORK.md). Live fleet: [TESTNET_STATUS.md](./TESTNET_STATUS.md).

---

## Current snapshot

| Area | State | Notes |
|------|--------|-------|
| Core protocol | **Green locally** | Workspace/fmt/Foundry v1.7.1 129/129 captured locally. Remote Actions deferred (billing). HEAD `53c8089` includes PRs 25–50. |
| In-repo hardening | **Through PR #50** | GOVERNANCE-API, SHIM-301, npm direct-dep gate, CLIENT-FRESHNESS, SUPPLY-CHAIN-CI. Next leftover protocol slice: WITNESSED-LOG (real witnesses only). |
| Public testnet stack | **Paused (cost)** | `api.testnet.cregnet.dev` treated as unreachable. Resume is operator work, not another protocol patch. |
| Waitlist | **Live** | Firebase — [WAITLIST-ONLY-HOSTING.md](./WAITLIST-ONLY-HOSTING.md) |
| Contribution Arena | **Separate product** | Educational/simulation surface — not protocol work. |
| External audit | **Open** | Freeze candidate `v0.4.3-testnet` — [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md) |
| CR-07 / HI-11 / ZK | **Implemented, disabled** | Do not flip flags from this repo. |

---

## Track A — Test the entire system

Run this before inviting external validators or resuming public hosting.

### A1. Local / CI verification (fast)

```powershell
cd chain-registry
cargo test --workspace
cd contracts
forge test
```

**Spot checks (Iteration 6):**

```powershell
cd chain-registry\crates\node
cargo test validate_
```

### A2. Local / fleet verification

| Step | Doc / script |
|------|----------------|
| Scale architecture | [TESTNET_SCALE_ARCHITECTURE.md](./TESTNET_SCALE_ARCHITECTURE.md) |
| Sepolia bootstrap smoke | `testnet.ps1 -RunSmokeTests`, `docker-compose.testnet.yml` |
| GCP observer/public services | `testnet/gcp/verify-public-services-profile.ps1`, `testnet/gcp/deploy-observer-pool.ps1`, `testnet/gcp/deploy-stack.ps1` |
| Real sandbox | `testnet/sandbox-301-verify.ps1` (`CREG_DEV_SANDBOX=false`) |
| Load / incentive soak | [testnet/SOAK_TEST.md](../chain-registry/testnet/SOAK_TEST.md) |

### A3. End-to-end publish → verified

| Step | Doc |
|------|-----|
| Sepolia smoke | [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) § Publish smoke |
| Publisher flow | [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) |
| Phase limits | [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) |

### A4. Resume public fleet (when ready)

```powershell
cd chain-registry
.\testnet\gcp\resume-testnet-hosting.ps1
.\testnet\hosting-301-verify.ps1 -BaseDomain testnet.cregnet.dev
```

Runbook index: [testnet/gcp/README.md](../chain-registry/testnet/gcp/README.md).

### A5. Incentive soak (Phase 4 — not started)

Harness: [testnet/SOAK_TEST.md](../chain-registry/testnet/SOAK_TEST.md), `testnet/soak-test/`.  
Goal: ≥1000 packages over days; monitor `creg_total_rewards_distributed`, `creg_last_incentives_refresh` on `/metrics`.

---

## Track B — Improve waitlist & Contribution Arena

These are **separate product surfaces** from the Rust node. They do not block consensus work but matter for public alpha UX.

### B1. Waitlist

| Task | Doc |
|------|-----|
| Hosting model (Firebase-only) | [WAITLIST-ONLY-HOSTING.md](./WAITLIST-ONLY-HOSTING.md) |
| Deploy / DNS | [WAITLIST_FIREBASE_DEPLOY.md](./WAITLIST_FIREBASE_DEPLOY.md) |
| Owner metrics (private) | [WAITLIST-ADMIN-MONITORING.md](./WAITLIST-ADMIN-MONITORING.md) |
| SPA source | External repo [Creg-waitlist](https://github.com/samuel-1-avson/Creg-waitlist) |

**Suggested improvements:** cohort export for genesis alpha, rate-limit tuning, admin stats hardening, align copy with [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md).

### B2. Contribution Arena

| Task | Doc |
|------|-----|
| Product overview | [CREG-Contribution-Arena/README.md](../CREG-Contribution-Arena/README.md) |
| Implementation status | [CREG-Contribution-Arena/docs/IMPLEMENTATION_STATUS.md](../CREG-Contribution-Arena/docs/IMPLEMENTATION_STATUS.md) |
| Production readiness | [CREG-Contribution-Arena/docs/PRODUCTION_READINESS.md](../CREG-Contribution-Arena/docs/PRODUCTION_READINESS.md) |
| Hub API migration | [CREG-Contribution-Arena/docs/HUB_INTEGRATION.md](../CREG-Contribution-Arena/docs/HUB_INTEGRATION.md) |
| Hub SQL migration | `chain-registry/hub-api/migrations/002_contribution_arena.sql` |

**Out of this workstream:** Contribution Arena is a separate educational/simulation product. Do not treat Arena launch, findings, or Hub API cutover as protocol work.

### B3. Hub API (shared backend)

Design: [TESTNET-HUB-DESIGN.md](./TESTNET-HUB-DESIGN.md).  
Deploy: `testnet/gcp/deploy-hub-api-cloudrun.ps1`.

---

## Track C — Prepare testnet phase (validator onboarding)

Use when moving from waitlist-only hosting back to **open validator participation**.

### C1. Operator checklist (external validators)

**Primary:** [VALIDATOR_ONBOARDING_CHECKLIST.md](./VALIDATOR_ONBOARDING_CHECKLIST.md)

Covers stake, dual keys, sandbox policy, scanner profile, registration API, and health checks.

### C2. Maintainer infrastructure

| Doc | Purpose |
|-----|---------|
| [TESTNET_SCALE_ARCHITECTURE.md](./TESTNET_SCALE_ARCHITECTURE.md) | **10k+ validator target topology** |
| [GENESIS_ALPHA_COHORT_PLAYBOOK.md](./GENESIS_ALPHA_COHORT_PLAYBOOK.md) | Wave 1 cohort (slots 1–10,000) |
| [testnet/OPERATOR.md](../chain-registry/testnet/OPERATOR.md) | External validators + GCP public-services runbook |
| [GCP-VALIDATOR-FLEET.md](./GCP-VALIDATOR-FLEET.md) | Optional seed validators on GCP |
| [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) | Key hygiene |

Legacy (do not use for new onboarding): [HYBRID-LOCAL-VALIDATORS.md](./HYBRID-LOCAL-VALIDATORS.md), [archive/3node-lab](../chain-registry/testnet/archive/3node-lab/README.md).

### C3. Resume hosting + DNS

1. `resume-testnet-hosting.ps1`  
2. `hosting-301-verify.ps1`  
3. Confirm `api.testnet.cregnet.dev` and explorer in chain spec  
4. Publish operator announcement with [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) link

### C4. Scale decentralization (Phase 3 — medium term)

- Genesis Alpha target: **10,000+** registered validators (batched waves)  
- Pilot 5–10 → Batch A/B per [GENESIS_ALPHA_COHORT_PLAYBOOK.md](./GENESIS_ALPHA_COHORT_PLAYBOOK.md)  
- Criteria in [WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md) Phase 3  
- Extend `creg doctor` decentralization checks (backlog)

---

## Track D — Security & audit (parallel)

| Priority | Item | Doc |
|----------|------|-----|
| P0 | Audit evidence pack | [WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md) Phase 0 |
| P0 | Vendor outreach | [SEC-401-VENDOR-OUTREACH.md](./SEC-401-VENDOR-OUTREACH.md) |
| P1 | Bridge/ZK hardening | Phase 2 in remediation plan; [L1_L2_HARDENING.md](./L1_L2_HARDENING.md) |
| P1 | Bridge soak | `CREG_BRIDGE_KEY` on validator-1; metrics on `/metrics` |

**Iteration 7 suggestion:** Phase 0 evidence pack + Phase 2.1 bridge resilience review.

---

## Suggested order (August 2026)

```mermaid
flowchart TD
    A[A1–A2 Local tests green] --> B{Resume public stack?}
    B -->|Yes| C[Track C: resume GCP + validator invites]
    B -->|No| D[Track B: waitlist + Arena polish]
    A --> E[Track D: audit evidence pack]
    C --> F[Track A4–A5: E2E + soak on live fleet]
    E --> G[SEC-401 vendor booking]
```

**Practical default:** Run **Track A1–A2** now (cheap). In parallel, **Track D** evidence pack. Resume **Track C** when you are ready to onboard validators; keep **Track B** for marketing surfaces.

---

## Documentation map

| Need | Document |
|------|----------|
| Protocol progress / testnet readiness | [PROTOCOL_PROGRESS.md](./PROTOCOL_PROGRESS.md) |
| Scale testnet topology | [TESTNET_SCALE_ARCHITECTURE.md](./TESTNET_SCALE_ARCHITECTURE.md) |
| Maintainer full index | [README.md](./README.md) |
| Public / external index | [PUBLIC.md](./PUBLIC.md) |
| Prioritized backlog | [NEXT_WORK.md](./NEXT_WORK.md) |
| Remediation phases | [WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md) |
| Issue registry | [DEEP_DIVE_ANALYSIS.md](../chain-registry/DEEP_DIVE_ANALYSIS.md) |
| Alpha gate tracker | [L2_PUBLIC_ALPHA_GATE_STATUS.md](./L2_PUBLIC_ALPHA_GATE_STATUS.md) |
| Historical / superseded | [archive/README.md](./archive/README.md) |

---

_Update this file when a track ships or hosting phase changes._
