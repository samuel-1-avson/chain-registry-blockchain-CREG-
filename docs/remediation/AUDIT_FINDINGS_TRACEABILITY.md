# Audit findings traceability

**Baseline:** [`CREG_COMPREHENSIVE_AUDIT_2026-07-10.md`](../CREG_COMPREHENSIVE_AUDIT_2026-07-10.md)  
**Status vocabulary:** Not started, Investigating, Confirmed, Design approved, Implementing, Test incomplete, Fixed, Independently reviewed, Accepted residual risk, Deferred with justification, Removed.

This register is deliberately conservative. A source edit is not marked Fixed until its tests, configuration, documentation, and operational rollout agree. `Implementation PR or commit` remains `working tree` until a reviewed commit exists.

| Finding | Severity | Status | Affected files / decision | Tests | Documentation | Residual risk |
|---|---|---|---|---|---|---|
| CR-01 | Critical | Implementing | `crates/common/src/package.rs`, `crates/node/src/package_admission.rs`, `crates/common/proto/node.proto`; dual-domain v2 payload added, strict production gate retained | New dual-proof tests pending; Cargo blocked | `protocol/IDENTITY_AND_OWNERSHIP.md` | Nonproduction compatibility mode still accepts legacy proof unless explicitly disabled |
| CR-02 | Critical | Test incomplete | `contracts/Registry.sol`, CLI, gRPC; ZK qualification retired and cannot be re-enabled | Foundry regression tests added; Forge unavailable | `protocol/PROTOCOL_SPECIFICATION.md`, ADR-009 | Existing deployed bytecode requires operator action |
| CR-03 | Critical | Test incomplete | `contracts/Registry.sol`, `crates/node/src/bridge.rs`; legacy batch path retired by default | Foundry regression tests added; Forge unavailable | ADR-003, `protocol/TRANSPARENCY_LOG.md` | Development-only research flag remains in source |
| CR-04 | Critical | Implementing | `package_admission.rs`, `validator_pipeline.rs`; admission committee is now reused by analysis | 1/4/7/50/51/100/1k/10k selection tests pending | ADR-006, `protocol/PROTOCOL_SPECIFICATION.md` | Full snapshot/VRF/assignment-proof schema remains open |
| CR-05 | Critical | Confirmed | `testnet/chain-spec.sepolia.json`; one validator/no bootnodes | Topology validation pending | `operations/DEPLOYMENT.md` | Current network is research alpha, not BFT |
| CR-06 | Critical | Implementing | `contracts/Staking.sol`, `contracts/Registry.sol`; per-release reservation and maturity window added | Fuzz test added; Forge unavailable | `economics/COST_AND_REWARD_MODEL.md` | Existing deployments do not gain storage changes automatically |
| CR-07 | Critical | Implementing | `chain_store.rs`, `pending_pool.rs`, `package_admission.rs`; namespace owner guard added | Ownership takeover/rotation tests pending | `protocol/IDENTITY_AND_OWNERSHIP.md` | Upstream authority/delegation adapter is not complete |
| CR-08 | Critical | Implementing | `contracts/Governance.sol`, `contracts/Registry.sol`; paused unpause proposal and emergency revoke added | Pause recovery test added; Forge unavailable | `governance/GOVERNANCE_MODEL.md` | Existing Governance deployments need migration |
| CR-09 | Critical | Deferred with justification | Custom PBFT remains research-only; no mainnet claim | Byzantine/partition suite pending | ADR-003, `protocol/PROTOCOL_SPECIFICATION.md` | Consensus safety is not production-assured |
| CR-10 | Critical | Not started | CLI/API freshness redesign | Rollback/freeze tests pending | `protocol/CLIENT_VERIFICATION.md` | Clients may still consume stale metadata |
| HI-11 | High | Investigating | `validator_set_history.rs`, `sync.rs` | Historical-set adversarial tests pending | `protocol/PROTOCOL_SPECIFICATION.md` | Long-range trust model incomplete |
| HI-12 | High | Implementing | `crates/consensus/src/evidence_votes.rs`; quorum now keys on profile/disposition, retains distinct evidence | Divergent-evidence tests updated; Cargo blocked | `protocol/EVIDENCE_SPECIFICATION.md` | Profile common-mode compromise remains possible |
| HI-13 | High | Implementing | `finalized_tx.rs`, block producer; buffer now applies backpressure instead of dropping finalized transactions | No-drop/restore tests pending; durable queue still required | `operations/DISASTER_RECOVERY.md` | Process loss still requires durable transactional queue |
| HI-14 | High | Not started | `contracts/Registry.sol` signature digest | EIP-712 replay tests pending | `protocol/PROTOCOL_SPECIFICATION.md` | Legacy validator signatures lack full domain binding |
| HI-15 | High | Design approved | Canonical source is target transparency log; current chain is legacy derived state | Rebuild/convergence tests pending | ADR-002/003, `architecture/TARGET_ARCHITECTURE.md` | Migration not implemented |
| HI-16 | High | Not started | Block producer/consensus resource limits | Load and oversized-block tests pending | `protocol/PROTOCOL_SPECIFICATION.md` | Unbounded block resource risk remains |
| HI-17 | High | Not started | Foundry/toolchain CI | Clean-checkout contract CI unavailable | `testing/TEST_STRATEGY.md` | Solidity changes are unverified here |
| HI-18 | High | Implementing | Governance constructor/cancel/pause changes | Governance invariants pending | `governance/GOVERNANCE_MODEL.md` | Signer independence and hardware custody require operations |
| HI-19 | High | Not started | `crates/cli/src/sbom.rs` | SBOM fixture/conformance tests pending | `protocol/EVIDENCE_SPECIFICATION.md` | Current output is incomplete |
| HI-20 | High | Not started | Provenance/build workers | Reproducibility corpus pending | ADR-014 | CREG does not yet verify submitted package builds |
| HI-21 | High | Design approved | Token rewards removed from MVP security path | Cost-accounting tests pending | `economics/COST_AND_REWARD_MODEL.md`, ADR-008 | Revenue model not validated |
| ME-22 | Medium | Not started | IPFS/object storage | Withholding/replication drills pending | `operations/DISASTER_RECOVERY.md` | CID does not guarantee availability |
| ME-23 | Medium | Deferred with justification | Shielded path remains disabled | Tenant isolation tests pending | ADR-013 | Single-node X25519 is not a threshold privacy system |
| ME-24 | Medium | Confirmed | Governance API returns deliberate 501 | API conformance pending | `architecture/DATA_FLOW.md` | UI/API may diverge from contract state |
| ME-25 | Medium | Not started | Web/contract CI | Unit/accessibility/Forge gates pending | `testing/TEST_STRATEGY.md` | Regressions can escape |
| ME-26 | Medium | Implementing | ZK/rollup/cross-chain/private feature freeze | Capability-matrix test pending | `remediation/IMPLEMENTATION_STATUS.md` | Legacy surfaces remain in repository |
| ME-27 | Medium | Confirmed | DDC inventory script | Real compiler-like target corpus needed | ADR-014 | Current no-target success is not DDC evidence |
| ME-28 | Medium | Implementing | Capability matrix and generated docs | Config/source consistency test pending | `remediation/IMPLEMENTATION_STATUS.md` | Deployment documentation can drift |
| LO-29 | Low | Not started | CLI publisher policy | Exact identity propagation test pending | `protocol/CLIENT_VERIFICATION.md` | Publisher allow/deny may be ineffective |
| LO-30 | Low | Not started | Explorer/index APIs | Rebuild/completeness tests pending | `architecture/DATA_FLOW.md` | Derived views may be incomplete |

## Review record

| Review date | Reviewer | Scope | Result |
|---|---|---|---|
| 2026-07-10 | Codex working-tree review | Source, contracts, configuration, tests, docs | No independent review; Cargo/Forge execution unavailable |
