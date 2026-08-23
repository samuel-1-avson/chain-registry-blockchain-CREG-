# Next work — prioritized backlog

**Updated:** 2026-08-23
**Planning hub:** [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md)  
**Context:** [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md), [WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md), [TESTNET_STATUS.md](./TESTNET_STATUS.md)

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
- **Network posture:** `chain-spec.sepolia.json` lists four coordinated
  validators (`validator-1`…`validator-4`) and empty bootnodes. That is the
  specified public-alpha topology, not a live multi-operator network and not
  Byzantine fault tolerance. The public GCP fleet is paused; see
  [WAITLIST-ONLY-HOSTING.md](./WAITLIST-ONLY-HOSTING.md) and
  [TESTNET_STATUS.md](./TESTNET_STATUS.md).
- **Out of this workstream:** Contribution Arena is a separate educational
  and simulation product. Protocol planning here does not include Arena
  launch, findings routing, or Hub cutover.
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
| 4 | **VAL-INDEPENDENCE** | blocked externally | Independent validators and bootnodes | Recruit and verify separately controlled operators/keys/infrastructure; a four-validator coordinated spec must not be represented as Byzantine-fault-tolerant. |
| 5 | **SPEC-ROTATE** | superseded | Repair signed Sepolia specification | Fresh-genesis candidate recorded 2026-08-01/02 in `chain-spec.sepolia.json` (genesis 2026-08-02). Remaining work is independent verification and public activation, not another in-repo rotate. |
| 6 | **SEC-401** | open | External security audit | Scope: [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md). Candidate freeze: `v0.4.3-testnet` (`3eb3a23…`). Vendor, start date, and signed findings remain TBD. Cut a later tag only if protocol diffs land after VERIFY-401. |
| 7 | **DOC-RELEASE** | active | Publish synchronized protocol documents | Public README/PUBLIC/install examples now pin `v0.4.3-testnet`. Still regenerate workflow/whitepaper PDFs and refresh manifest hashes. |
| 8 | **TESTNET-RESUME** | gated | Resume public fleet only after operator gates | Use `testnet/gcp/resume-testnet-hosting.ps1` and `hosting-301-verify.ps1` only after independent validators, checkpoint, trust, and rehearsal sign-offs. |

---

## P1 — Quality (next 2–4 weeks)

| # | ID | Status | Task | Acceptance |
|---|-----|--------|------|------------|
| 8 | **EIP712-401** | active | Epoch-bound contract signatures | Finish validator-set epoch binding for verdict and admission signatures, adversarial replay cases, and contract review. |
| 9 | **ARCHIVE-401** | repository complete; ops open | Authority evidence resource controls | Exercise pressure GC/restore; add capacity alerts, shared-filesystem coordination decision, backup, and legal-hold policy. |
| 10 | **BRIDGE-301** | open | L1 bridge soak | `CREG_BRIDGE_KEY` on validator; batch observed or documented skip + alert. |
| 11 | **SOAK-301** | open | Incentive/load soak | Run [testnet/SOAK_TEST.md](../chain-registry/testnet/SOAK_TEST.md) and availability/DR drills. |
| 12 | **SEC-402** | open | Network partition chaos | Run `k8s/55-network-partition-test.yaml`, collect traces, and publish a postmortem. |
| 13 | **ISSUE-013** | done (2026-08-22) | Fail-closed sandbox bypass | Merged in PR #26: crate-level bypass requires `CREG_TESTNET` and blocks public/production profiles. |

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
| DIST-401 | 2026-08-03 | `v0.4.3-testnet` binaries (current public pin) |
| HOSTING-301 | 2026-06-10 | `testnet.cregnet.dev` (resumable; currently paused) |
| SPEC-ROTATE | 2026-08-02 | Fresh genesis candidate in `chain-spec.sepolia.json`; public activation still gated |
| DOC-301 | 2026-06-08 | Doc index rationalization |
| GAPS-Iter5 | 2026-06-26 | Gossip/PBFT hardening |
| GAPS-Iter6 | 2026-06-26 | Batch + finalize relay forge tests |
| HI11-REPO | 2026-07-29 | Certified transitions, gossip validation, checkpoint ancestry, mandatory gate, reorg/retention tests; live activation still pending |
| CR07-ARCHIVE | 2026-07-29 | Bounded evidence/artifact archive with minimum-retention pressure GC; operational drills still pending |
| ISSUE-013 | 2026-08-22 | `CREG_DEV_SANDBOX` requires testnet profile; public/production blocked (PR #26) |
| ISSUE-010 | 2026-08-22 | CrossChain `AxelarAdapter.setChainName` is governance-gated; adapter remains undeployed |
| ISSUE-015 | 2026-08-22 | WASM WASI stubs gated by `CapabilitySet` / `ResourceLimits`; stdout capture via `fd_write` |
| ISSUE-011 | 2026-08-23 | Publisher ZK admission placeholders retired; gRPC rejects leftover 85/`true` proofs |
| ISSUE-016 | 2026-08-23 | Historical sync verifies `validator_set_hash` against the set at that height; HI-11 checkpoint still operational |
| ISSUE-017 | 2026-08-23 | `GET /v1/governance/proposals` mirrors Governance.sol; 200 empty if unconfigured, 503 on L1 RPC failure |
| ISSUE-012 | 2026-08-23 | Deep scan fails closed on corrupt/timeout input; votes carry the actual `degraded-*` scanner version |

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
