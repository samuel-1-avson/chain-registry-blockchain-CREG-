# CREG Weaknesses / Risks / Gaps Remediation Plan

**Date:** 2026-06-26  
**Source:** SYSTEM_RATING_2026-06-26.md (post-incentives and Tier-1 work)  
**Reference Documents:**
- `CREG_SYSTEM_WORKFLOW.md` §15 (Architecture Improvement Roadmap & Iterative Execution)
- `chain-registry/DEEP_DIVE_ANALYSIS.md` (Issue Registry)
- `docs/REMEDIATION_BACKLOG.md`
- `docs/SEC-401-AUDIT-SCOPE.md` and related SEC-401 docs
- `docs/NEXT_WORK.md`, `docs/L1_L2_HARDENING.md`
- `WHITEPAPER.md` (current v0.3.0)

**Guiding Principles (from §15):**
- Work iteratively (1–3 focused items per iteration).
- Update this plan + workflow §15 + DEEP_DIVE status after every significant step.
- Never regress BFT safety, determinism, or the low-reward 42M supply model.
- Prioritize items that unblock audit closure and mainnet claims.

## 1. Gap Summary (from System Rating)

1. **External audit still pending** — Highest priority blocker for stronger claims.
2. **Validator set remains coordinated** — Not yet large/permissionless/independent.
3. **Bridge + ZK + cross-layer surfaces** — Most complex and highest-risk areas (even after recent hardening).
4. **Remaining DEEP_DIVE items** — Gossip fallbacks (002/006/007), BatchOperations publisher identity (003), full finalize relay controls (008) have partial progress but need final closure + re-review.
5. **End-to-end incentive soak testing** — Relatively new; needs realistic load validation.

## 2. Overall Phasing & Prioritization

**Phase 0 (Immediate / Parallel):** Audit Acceleration  
**Phase 1 (High):** Close Remaining Concrete DEEP_DIVE Code Items + Relay Controls  
**Phase 2 (High):** Bridge + ZK + Cross-Layer Hardening & Re-Review  
**Phase 3 (Medium):** Validator Set Decentralization Path  
**Phase 4 (Medium/Operational):** End-to-End Incentive Soak Testing & Validation  

Each phase should produce:
- Code/config changes (with tests)
- Updated DEEP_DIVE issue status
- Workflow §15 iteration log entry
- Metrics / observability improvements where relevant
- Updated remediation backlog entry

## Phase 0: Audit Acceleration (Highest Priority — Start Immediately)

**Goal:** Make the project audit-ready and kick off SEC-401 (or equivalent) as fast as possible.

**Tasks:**
1. Review and finalize `docs/SEC-401-AUDIT-SCOPE.md` (update with current state of ZK binding, `enforceFinalizeRelays`, incentives monitoring, committee snapshots, and recent fixes).
2. Update `docs/SEC-401-outreach-ready.md` and `SEC-401-VENDOR-OUTREACH.md` with latest architecture docs (point to current WHITEPAPER + workflow §9.11/9.12/15).
3. Produce an "Audit Evidence Pack":
   - Current DEEP_DIVE issues with status (fixed / mitigated / open).
   - List of all recent changes since June 2026 baseline (ZK binding, bridge safety, reward emission instrumentation, performance snapshots, etc.).
   - Test results (forge test, cargo test summaries).
   - Threat model / STRIDE summary focused on Bridge/ZK/finalize paths.
4. Prepare a clean remediation status for auditors (use/extend `docs/REMEDIATION_BACKLOG.md`).
5. Add explicit "Audit Readiness" gate to `docs/L2_PUBLIC_ALPHA_GATE_STATUS.md` or `TESTNET_PHASE_SCOPE.md`.

**Verification:**
- All docs reference accurate current contract addresses / code hashes (use deployments/ and chain-spec).
- One-pager summary of "what has changed since last vendor conversation".

**Success Criteria:**
- Audit scope is frozen and vendor engagement is active or scheduled.
- All open high/critical items from DEEP_DIVE have a clear "in audit" or "pre-audit closure" status.

**Owner Suggestion:** Security lead + one engineer for evidence pack.  
**Effort:** 1–2 weeks focused work (parallel with other phases).

## Phase 1: Close Remaining DEEP_DIVE Code Items + Relay Controls

Focus on items that are mostly code and have partial progress.

### 1.1 Gossip Signature & Validation Hardening (DEEP_DIVE ISSUE-002, 006, 007)

**Current State (as of 2026-06-26):**
- `crates/node/src/validator_pipeline.rs` has improved `hex::decode(privkey_str)` error handling.
- `validate_vote_gossip_message` exists.
- PBFT Prepare path still noted as simplified in some comments.

**Tasks:**
1. Audit all remaining `unwrap_or_default` / `unwrap_or(vec![])` paths in gossip and validator key loading (p2p.rs, validator_pipeline.rs, validator_set_sync.rs, etc.).
2. Ensure **all** gossip vote and PBFT Prepare/Commit messages go through the full domain-separated verification in `consensus::pbft`.
3. Add or strengthen tests:
   - Invalid privkey at startup → clear fatal log + graceful degradation (do not start validator).
   - Malformed gossip vote → rejected with metric `creg_gossip_rejected_*`.
   - PBFT Prepare with wrong signature domain → rejected.
4. Add Prometheus metrics for gossip validation failures and key loading errors.
5. Update `p2p_rate_limit.rs` and ban logic if needed for repeated bad signatures.

**Files to touch:**
- `crates/node/src/p2p.rs`
- `crates/node/src/validator_pipeline.rs`
- `crates/consensus/src/pbft.rs`
- `crates/node/src/metrics.rs` (new gauges)

**Verification:**
- `cargo test` (focus on p2p + consensus tests).
- Manual: Start node with bad `CREG_VALIDATOR_KEY` → clear error, no silent bad votes.
- Fuzz or property test for signature domains if possible.

### 1.2 BatchOperations Publisher Identity — Final Verification (DEEP_DIVE ISSUE-003)

**Current State:**
- `BatchOperations.sol` now calls `registry.submitPackageFor(publisher, ...)` and passes `msg.sender` as the real publisher.
- `submitPackageFor` exists in Registry.sol.

**Tasks:**
1. Full review of `submitPackageFor` implementation in Registry.sol (stake check uses `publisher` parameter correctly).
2. Add forge tests in `contracts/test/` that:
   - Call batch as a non-staked contract → fails.
   - Call batch as staked publisher → succeeds with correct publisher recorded.
3. Wire any missing reward/ reputation attribution that should go to the real publisher.
4. Document the intended use of BatchOperations in contracts/README.md and workflow.

**Verification:** New forge tests pass + manual Sepolia test (if needed).

### 1.3 Finalize Relay Controls & Replay Protection (DEEP_DIVE ISSUE-008)

**Current State:**
- `enforceFinalizeRelays = true` default.
- `packageFinalizeRelays` allowlist mapping.
- `finalizePackage` checks the allowlist when enforced.

**Tasks:**
1. Decide and implement one of:
   - Keep current allowlist model (recommended for testnet) and make it the documented default.
   - Add per-package nonce or round identifier to `_sigDigest` to prevent replay of old signature sets.
2. Expose management functions (add/remove relay) only via governance.
3. Add tests (forge) for:
   - Unauthorized caller when `enforceFinalizeRelays=true` → reverts.
   - Stale signatures (if nonce added).
4. Update whitepaper + workflow to clearly describe the relay model and when the flag can be relaxed.

**Files:**
- `chain-registry/contracts/Registry.sol`
- `contracts/test/Registry.t.sol` (or new test)

## Phase 2: Bridge + ZK + Cross-Layer Surfaces (Highest Technical Risk)

**Goal:** Reduce blast radius and increase confidence in the most complex integration points.

**Sub-areas:**

### 2.1 Bridge Robustness
- Review remaining `expect` / `unwrap` in `crates/node/src/bridge.rs` and retry logic.
- Ensure bridge task never crashes the whole node (run as resilient background task with health reporting).
- Add metrics: `creg_bridge_batches_submitted`, `creg_bridge_errors`, last successful batch time.
- Improve startup validation of `CREG_ETH_RPC`, governance threshold warnings (already partially present).
- Consider circuit breaker: auto-disable bridging on repeated failures + operator alert.

### 2.2 ZK Binding & Circuit Confidence
- Confirm the **circuit itself** (not just Registry binding) now constrains content/manifest hashes (follow up on DEEP_DIVE ISSUE-001).
  - If circuit was not regenerated, schedule trusted setup + redeploy of verifying key.
- Add end-to-end test: publish package → ZK fast path (if enabled) → verify binding on-chain.
- Document exactly which paths use ZK vs PBFT (in whitepaper and workflow).

### 2.3 Cross-Layer Consistency
- Ensure that packages finalized via PBFT + `finalizePackage` (with relay) produce the same on-chain state as ZK path.
- Add invariants / tests around `validationMode` (PBFT vs ZKProof).
- Review light-client proof paths for consistency with L1 Registry state.

**Verification for Phase 2:**
- Run full local testnet with bridge enabled.
- Inject faults (bad RPC, missing ZK keys, invalid proofs) → system degrades gracefully.
- New integration test that exercises bridge batch + per-package finalize.

## Phase 3: Validator Set Decentralization

**Goal:** Clear path from current coordinated set → permissionless / independent validators.

**Tasks:**
1. Review and improve validator onboarding experience:
   - `VALIDATOR_ONBOARDING_CHECKLIST.md`
   - `HYBRID-LOCAL-VALIDATORS.md`
2. Enhance public validator set sync and reputation visibility so new operators can evaluate current set.
3. Define and publish "Independent Validator Criteria" (stake minimums, uptime, sandbox enforcement, key management, geographic diversity).
4. Add tooling for operators:
   - `creg doctor` extended with decentralization health.
   - Public dashboard of active validator performance / contribution counts.
5. Governance process for adding new validators (or moving to on-chain application if VRF committee selection matures).
6. Test with at least one external / community validator in the testnet.

**Milestones:**
- At least 5–7 independent validators running with real stake.
- Documented process for new operators to join.

## Phase 4: End-to-End Incentive Soak Testing

**Goal:** Validate the new low-base reward system + performance feedback loop under realistic load.

**Tasks:**
1. Create or enhance a soak / load test harness:
   - Script or docker profile that publishes hundreds/thousands of packages over hours/days.
   - Mix of good/bad/malicious fixtures.
   - Multiple validators (mandatory + voluntary).
2. Instrument and monitor:
   - Reward emission success/failure counters
   - `last_incentives_refresh`
   - Treasury draw
   - Performance score evolution (before/after finalization)
   - Committee selection distribution
3. Run with realistic pinning (ipfs-pinner sidecar enabled).
4. Measure:
   - Does the 0.00001 base + points system behave as modeled?
   - Any unexpected treasury depletion?
   - Performance score stabilizes or drifts incorrectly?
   - L1 gas costs for `distributeWorkRewards` calls.
5. Produce a report (add to workflow §15 or new `INCENTIVES_SOAK_REPORT.md`).
6. Add automated or semi-automated regression for incentives in CI if feasible.

**Success Criteria:**
- Soak run of ≥ 1000 packages across multiple days with all metrics green.
- Documented expected vs observed behavior.
- No silent failures in reward path.

## 3. Cross-Cutting Work

- **Documentation:** After every phase, update:
  - `CREG_SYSTEM_WORKFLOW.md` §15 (new iteration log entry)
  - `chain-registry/DEEP_DIVE_ANALYSIS.md` (mark issues fixed/mitigated)
  - `docs/REMEDIATION_BACKLOG.md`
  - `WHITEPAPER.md` risk / status sections if material changes
- **Testing:** Increase coverage for the changed surfaces (especially L1 finalize paths and gossip).
- **Observability:** Ensure every major surface has metrics and clear log messages.
- **Risk Register:** Maintain an explicit list of "known residual risks post-remediation".

## 4. Suggested Iteration Cadence (example)

- Iteration 5: Phase 0 (Audit pack) + Phase 1.1 (Gossip)
- Iteration 6: Phase 1.2 + 1.3 (Batch + Relays)
- Iteration 7: Phase 2 start (Bridge/ZK review + tests)
- Parallel track: Phase 0 vendor outreach + Phase 4 soak test setup
- Later iterations: Phase 3 decentralization + full Phase 2 closure + Phase 4 results

## 5. Success Definition for This Plan

- External audit is booked or in progress.
- All listed DEEP_DIVE issues from the rating have "Closed" or "Mitigated + Audited" status.
- Bridge/ZK paths have significantly more test coverage and defensive code.
- Clear, published path to independent validator set.
- At least one public soak report showing incentive system behaving as designed under load.
- Whitepaper and workflow accurately reflect the hardened state.

---

**Next Action Recommendation:**

Start with **Phase 0** (audit) in parallel with **Phase 1** (gossip + relay closure), as these two unblock the most.

Update this plan and the workflow §15 after the first iteration on any of these gaps.

This plan is intended to be living — merge it with or reference from `REMEDIATION_BACKLOG.md` and `NEXT_WORK.md`.

---

## 6. Execution Status (living)

**Iteration 6 completed:** 2026-06-26 — Phase 1.2 (BatchOperations) + Phase 1.3 (finalize relays).

| Phase / item | Status | Notes |
|--------------|--------|-------|
| Phase 0 — Audit scope refresh | in-progress | `SEC-401-AUDIT-SCOPE.md` updated with gossip/PBFT validation boundary; evidence pack pending |
| Phase 0 — Audit evidence pack | pending | DEEP_DIVE status rollup + test summaries not yet assembled |
| Phase 1.1 — Gossip vote validation | **mitigated** | Iteration 5 — `validate_vote_gossip_message`, startup gate, metrics, unit tests |
| Phase 1.1 — PBFT Prepare/Commit validation | **mitigated** | Iteration 5 — full domain-separated verify on inbound gossip |
| Phase 1.2 — BatchOperations publisher | **done** | Forge: authorized relay, unstaked reject, allowlist reject; `contracts/README.md` |
| Phase 1.3 — Finalize relay controls | **mitigated** | Default `enforceFinalizeRelays=true`; relay allowlist + duplicate-sig replay tests; README |
| Phase 1.3 — Per-package sig nonce | deferred | Allowlist model retained for testnet; nonce in `_sigDigest` optional follow-up |
| Gap 2 — Validator decentralization | open | — |
| Gap 3 — Bridge/ZK surfaces | open | On-chain ZK limb binding + test helper aligned (30/30 Registry forge); circuit regen scheduled |
| Gap 5 — Incentive soak | open | Observability counters exist; soak harness pending |

**DEEP_DIVE updates (Iteration 6):**
- ISSUE-003: **fixed** — `submitPackageFor` + forge coverage.
- ISSUE-008: **mitigated** — relay allowlist default + replay guards tested.
- ISSUE-002 / 006 / 007: **mitigated** (Iteration 5, unchanged).

**Verification (Iteration 6):**
- `forge test --match-contract RegistryTest` — **30/30 pass** (incl. batch + relay tests).
- Full contracts suite — **37/37 pass** (Registry + CrossChainRegistry).
- ZK test helper `_zkInputs` aligned with `Registry._contentHashToLimbs` binding.

**Next iteration (7):** Phase 0 evidence pack; Phase 2 start (bridge/ZK integration tests).