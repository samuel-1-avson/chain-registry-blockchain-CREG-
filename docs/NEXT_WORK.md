# Next work — prioritized backlog

**Updated:** 2026-07-29
**Planning hub:** [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md)  
**Context:** [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md), [WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md)

---

## Current phase

- **Protocol hardening:** CR-07 namespace authority and CR-04/HI-11
  validator-history protections are implemented at repository level but remain
  disabled pending trust material, a real checkpoint, and independent
  operational evidence.
- **Verification:** focused node, full node (209/209), CLI (41/41), verifier,
  activation-manifest, and local contract (111/111) suites pass. The full Rust
  all-target run, rustfmt cleanup, pinned Foundry v1.7.1 run, and document
  regeneration are still active gates.
- **Network posture:** the recorded one-validator/no-bootnode topology is
  research alpha and has no Byzantine fault tolerance. The public GCP fleet may
  be stopped to save cost; see
  [WAITLIST-ONLY-HOSTING.md](./WAITLIST-ONLY-HOSTING.md).
- **Activation posture:** do not enable authoritative recovery or mandatory
  certified committees until the signed activation/checkpoint ceremonies and
  multi-operator rehearsals are complete.

---

## P0 — Gating work

| # | ID | Status | Task | Acceptance |
|---|-----|--------|------|------------|
| 1 | **VERIFY-401** | active | Finish reproducible repository verification | Complete the current Rust all-target run, private-temp sidecar integration, rustfmt/check, affected reruns, pinned Foundry v1.7.1, and clean CI evidence. |
| 2 | **CR07-ACTIVATE** | blocked externally | Production namespace-authority package | Reviewed npm/PyPI policy and trust roots, official npm authentication-mode signal, positive fixtures, signed manifest, platform artifacts, and multi-operator rehearsal. |
| 3 | **HI11-ACTIVATE** | blocked externally | Certified-history activation | Independently verified checkpoint plus replay, partition, reorg, rebuilt-node, long-range, and process-kill rehearsals before mandatory mode. |
| 4 | **VAL-INDEPENDENCE** | blocked externally | Independent validators and bootnodes | Recruit and verify separately controlled operators/keys/infrastructure; a one-validator topology must not be represented as Byzantine-fault-tolerant. |
| 5 | **SPEC-ROTATE** | blocked externally | Repair signed Sepolia specification | Rotate repository-disclosed signing/validator material, decide intended genesis/topology, recompute, sign with an approved secret, independently verify, then mirror exact canonical bytes/signature. Current canonical genesis and both static mirrors are invalid/drifted and must not be published. |
| 6 | **SEC-401** | open | External security audit | Scope: [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md). Vendor, independently controlled reviewer, start date, and signed findings remain TBD. |
| 7 | **DOC-RELEASE** | active | Publish synchronized protocol documents | Regenerate workflow/whitepaper PDFs, synchronize public and Functions mirrors, visually inspect every page, and refresh manifest hashes. |
| 8 | **TESTNET-RESUME** | gated | Resume public fleet only after operator gates | Use `testnet/gcp/resume-testnet-hosting.ps1` and `hosting-301-verify.ps1` only after spec/key repair, migrations, validators, checkpoint, trust, and rehearsal sign-offs. |

---

## P1 — Quality (next 2–4 weeks)

| # | ID | Status | Task | Acceptance |
|---|-----|--------|------|------------|
| 8 | **EIP712-401** | active | Epoch-bound contract signatures | Finish validator-set epoch binding for verdict and admission signatures, adversarial replay cases, and contract review. |
| 9 | **ARCHIVE-401** | repository complete; ops open | Authority evidence resource controls | Exercise pressure GC/restore; add capacity alerts, shared-filesystem coordination decision, backup, and legal-hold policy. |
| 10 | **BRIDGE-301** | open | L1 bridge soak | `CREG_BRIDGE_KEY` on validator; batch observed or documented skip + alert. |
| 11 | **SOAK-301** | open | Incentive/load soak | Run [testnet/SOAK_TEST.md](../chain-registry/testnet/SOAK_TEST.md) and availability/DR drills. |
| 12 | **SEC-402** | open | Network partition chaos | Run `k8s/55-network-partition-test.yaml`, collect traces, and publish a postmortem. |
| 13 | **ARENA-HUB** | open | Contribution Arena → Hub API | Apply `002_contribution_arena.sql`; [HUB_INTEGRATION.md](../CREG-Contribution-Arena/docs/HUB_INTEGRATION.md). |

---

## P2 — Soon (not blocking)

| # | ID | Task |
|---|-----|------|
| 14 | **WITNESSED-LOG** | Replace legacy/local history authority with a witnessed append-only log and prove derived-index completeness. |
| 15 | **CLIENT-FRESHNESS** | Implement threshold-signed freshness metadata, expiry, anti-rollback state, and witnessed inclusion. |
| 16 | **SUPPLY-CHAIN-CI** | Add SBOM, artifact provenance, dependency policy, web/E2E CI, and release verification. |
| 17 | **GOVERNANCE-API** | Complete authenticated governance API and adversarial authorization tests. |
| 18 | **REM-211+** | Complete resource metrics and Grafana dashboards per [observability/README.md](../chain-registry/observability/README.md). |
| 19 | **VAL-SCALE** | Expand a verified independent validator cohort in capacity-tested batches; waitlist size is not validator capacity. |
| 20 | **ZK-CIRCUIT** | Complete circuit/contract review and regenerate setup only if the binding changes; do not restore the retired qualification claim. |
| 21 | **SHIM-301** | Resolve shim security and freshness behavior relative to `creg install`. |

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
| HI11-REPO | 2026-07-29 | Certified transitions, gossip validation, checkpoint ancestry, mandatory gate, reorg/retention tests; live activation still pending |
| CR07-ARCHIVE | 2026-07-29 | Bounded evidence/artifact archive with minimum-retention pressure GC; operational drills still pending |

---

## Quick commands

```powershell
cd chain-registry

# Local verification (run from chain-registry/)
cargo fmt --all -- --check
cargo test --locked --workspace --all-targets -j1
forge test

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
