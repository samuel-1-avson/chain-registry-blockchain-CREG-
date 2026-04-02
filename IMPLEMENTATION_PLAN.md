# Chain Registry: Phased Implementation Plan

> **Based on:** DEEP_DIVE_ARCHITECTURE_REPORT.md v2.0  
> **Current Score:** 6.0/10 | **Target Score:** 8.1/10  
> **Created:** 2026-04-01

---

## Executive Summary

This document translates the architectural roadmap from the Deep-Dive Report into an actionable, tracked implementation plan. We proceed through **5 phases** over **24 months**, prioritizing security fixes (P0) before features (P2–P4).

**Immediate focus:** Phase 1 (Months 1–3) — Foundation & Critical Security Fixes.

---

## Phase 1: Foundation & Critical Security Fixes (Months 1–3)

### Updated Reality-Based Deliverables

After code inspection, several reported issues have been partially addressed or mischaracterized in the report. This table reflects the *actual* current state:

| # | Deliverable | Priority | Status | Actual Effort | Notes |
|---|---|---|---|---|---|
| 1.1 | **Fix vote endpoint auth gap** | P0 | **OPEN — Critical** | 1 day | Code verifies Ed25519 sigs, BUT `validator_pubkey` is not bound to `validator_id`. Anyone can vote for any validator using their own keypair. |
| 1.2 | **Remove hardcoded fallback IPFS CID** | P0 | **NOT FOUND** | 0.5 days | Hardcoded CID `bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi` not present in current codebase. Verified via grep. Considered resolved. |
| 1.3 | **Publisher key rotation transaction type** | P0 | **OPEN** | 1 week | No `RotatePublisherKey` transaction exists. Critical for key compromise recovery. |
| 1.4 | **Replace `.unwrap()` in production code** | P1 | **OPEN** | 2 days | ~20 occurrences in production paths (`block.rs`, `bridge.rs`, `main.rs`, `p2p_rate_limit.rs`, `rate_limit.rs`, `validator_pipeline.rs`). Tests excluded. |
| 1.5 | **Add `nonReentrant` to Staking.sol & Governance.sol** | P1 | **OPEN** | 0.5 days | `distributeSlashPool()` and `execute()` lack reentrancy guards. |
| 1.6 | **`decrypt_shielded()` implementation** | P1 | **ALREADY DONE** | 0 days | Fully implemented in `validator_pipeline.rs`. Threshold-encryption crate is also complete. Report was outdated. |
| 1.7 | **Rate limiting middleware** | P1 | **ALREADY DONE** | 0 days | Full sliding-window implementation exists in `node/src/rate_limit.rs` with per-endpoint limits. |
| 1.8 | **PostgreSQL sync worker (sled → PostgreSQL ETL)** | P1 | **OPEN** | 2 weeks | No standalone sync service exists. Need to build `crates/db-sync` or equivalent. |
| 1.9 | **Expand typosquat list (90 → 10,000 packages)** | P1 | **OPEN** | 2 days | Currently hardcoded ~90 packages in `validator/src/typosquat.rs`. Need dynamic loading from dataset. |
| 1.10 | **IPFS pinning reward module** | P1 | **OPEN** | 2 weeks | No CREG reward mechanism for mirror nodes exists yet. |
| 1.11 | **Deploy testnet with 10+ validators** | Required | **OPEN** | 2 weeks | Gate for external security audit. |

### Phase 1 Implementation Order

**Week 1–2: Security Hardening (P0 + P1 quick wins)**
1. Fix vote endpoint auth gap (`api.rs`)
2. Add `nonReentrant` modifiers to Solidity contracts
3. Replace critical `.unwrap()` calls in production Rust code
4. Implement publisher key rotation transaction type

**Week 3–4: Validation & Data Layer**
5. Expand typosquat list with top-10,000 packages per ecosystem
6. Begin PostgreSQL sync worker architecture and implementation

**Week 5–6: Infrastructure & Incentives**
7. Implement IPFS pinning reward module
8. Harden rate limiting (already functional, add Redis backing)

**Week 7–8: Testnet Deployment**
9. Deploy testnet with 10+ independent validators
10. Integration testing and bug fixes

---

## Phase 2: Package Validation Enhancement (Months 3–6)

| # | Deliverable | Priority | Est. Effort |
|---|---|---|---|
| 2.1 | Complete VRF-based validator selection | P2 | 2 weeks |
| 2.2 | ZK slashing evidence circuit (double-sign proof) | P2 | 4 weeks |
| 2.3 | AI deep learning malware scanner (CodeBERT + ONNX) | P2 | 6 weeks |
| 2.4 | Multi-signature package publishing (2-of-3) | P2 | 2 weeks |
| 2.5 | Package namespace reservation | P2 | 1 week |
| 2.6 | Commit-reveal voting | P2 | 2 weeks |
| 2.7 | Enhanced differential analysis (deep dep-graph diff) | P2 | 2 weeks |
| 2.8 | Quarantine status for borderline packages | P2 | 1 week |
| 2.9 | Package appeal CLI | P2 | 1 week |
| 2.10 | On-chain validator performance tracking | P2 | 1 week |

---

## Phase 3: Governance, Tokenomics & Developer Experience (Months 6–10)

| # | Deliverable | Priority | Est. Effort |
|---|---|---|---|
| 3.1 | Full DAO governance launch | P3 | 4 weeks |
| 3.2 | Treasury management system | P3 | 2 weeks |
| 3.3 | DEX liquidity bootstrapping | P3 | 1 week |
| 3.4 | Package dependency graph explorer | P3 | 3 weeks |
| 3.5 | CDN acceleration layer | P3 | 2 weeks |
| 3.6 | Package rollback system | P3 | 2 weeks |
| 3.7 | Offline verification mode | P3 | 3 weeks |
| 3.8 | DID integration (W3C DID) | P3 | 4 weeks |
| 3.9 | Cross-chain CCIP bridge | P3 | 6 weeks |
| 3.10 | ZK-compressed audit trail | P3 | 4 weeks |
| 3.11 | Arweave permanent archive integration | P3 | 1 week |
| 3.12 | Longer timelocks for critical params | P1 | 2 days |

---

## Phase 4: Advanced Features (Months 10–16)

| # | Deliverable | Priority | Est. Effort |
|---|---|---|---|
| 4.1 | ZK-based private validation | P4 | 8 weeks |
| 4.2 | IBC light client (Cosmos) | P4 | 8 weeks |
| 4.3 | Automated dispute resolution with ML | P4 | 4 weeks |
| 4.4 | Reputation NFTs | P4 | 2 weeks |
| 4.5 | Enterprise SLA guarantees | P4 | 4 weeks |
| 4.6 | PBFT → HotStuff migration | P4 | 8 weeks |
| 4.7 | Comprehensive observability | P4 | 2 weeks |
| 4.8 | Formal external security audit | Required | 4 weeks |

---

## Phase 5: Production Scaling (Months 16–24)

| # | Deliverable | Priority | Est. Effort |
|---|---|---|---|
| 5.1 | sled → TiKV distributed storage | P5 | 8 weeks |
| 5.2 | Quantum-resistant signature planning | P5 | 4 weeks |
| 5.3 | Multi-region validator distribution | P5 | Ongoing |
| 5.4 | Parallel IPFS fetching optimization | P5 | 1 week |
| 5.5 | Enterprise support program | P5 | Business |
| 5.6 | Ecosystem integrations (GitHub Actions, VS Code) | P5 | 4 weeks |
| 5.7 | Regulatory compliance (GDPR, SOC2) | P5 | 4 weeks |
| 5.8 | Bug bounty program (Immunefi) | Required | Ongoing |

---

## Critical Security Issues — Detailed Fix Plans

### Issue 1: Vote Endpoint Auth Gap (`api.rs`)

**Current broken behavior:**
```rust
// receive_vote() checks:
1. validator_id is in the active set
2. signature is valid Ed25519 over the message
// BUT: it does NOT check that validator_pubkey == the registered pubkey for validator_id
```

**Attack:** Attacker generates their own Ed25519 keypair, sets `validator_id = "honest-validator-1"`, and submits a correctly signed vote using their own key. The signature verifies, and the vote is accepted.

**Fix:** Add a validator pubkey registry lookup. The `NodeState.validator_set` or a new mapping must store `validator_id → validator_pubkey`. Before accepting a vote, verify:
```rust
let expected_pubkey = s.validator_set.pubkey_for(&vote.validator_id)
    .ok_or_else(|| reject)?;
if vote.validator_pubkey != expected_pubkey {
    return reject("Validator pubkey mismatch");
}
```

---

### Issue 2: Publisher Key Rotation

**New transaction type:**
```rust
// In common/src/block.rs Transaction enum:
RotatePublisherKey {
    canonical_prefix: String,    // e.g., "npm:lodash"
    old_pubkey: String,
    new_pubkey: String,
    sig_from_old: String,        // Ed25519 sign(new_pubkey) with old key
    sig_from_new: String,        // Ed25519 sign(old_pubkey) with new key
    timestamp: DateTime<Utc>,
}
```

**Validation rules:**
1. `old_pubkey` must currently own at least one package matching `canonical_prefix`
2. `sig_from_old` must verify: `Ed25519_verify(old_pubkey, new_pubkey, sig_from_old)`
3. `sig_from_new` must verify: `Ed25519_verify(new_pubkey, old_pubkey, sig_from_new)`
4. `timestamp` must be within ±5 minutes of now (replay protection)
5. Atomically update all matching `ChainRecord.publisher_pubkey` entries

---

### Issue 3: Reentrancy in Solidity

**Staking.sol:** Add `nonReentrant` modifier to:
- `distributeSlashPool()`
- `slashSeverity()`
- `slash()`
- `rejectValidator()`
- `withdrawValidatorStake()`
- `unstakeAsPublisher()`

**Governance.sol:** Add `nonReentrant` modifier to:
- `execute()` (via `_execute`)
- `vote()` (because it auto-calls `_execute`)

Implementation pattern: Use OpenZeppelin-style `ReentrancyGuard` or a simple custom mutex.

---

## File Change Map (Phase 1)

| File | Change |
|---|---|
| `crates/node/src/api.rs` | Bind vote pubkey to validator_id; add key rotation endpoint |
| `crates/common/src/block.rs` | Add `RotatePublisherKey` to `Transaction` enum |
| `crates/common/src/lib.rs` | Add `Validator.pubkey` field |
| `crates/node/src/chain_store.rs` | Handle `RotatePublisherKey` in block execution |
| `crates/node/src/main.rs` | Fix `.unwrap()` in gRPC address parsing |
| `crates/node/src/bridge.rs` | Fix `.unwrap()` in RPC provider setup |
| `crates/node/src/rate_limit.rs` | Replace `.unwrap()` on mutex locks |
| `crates/node/src/p2p_rate_limit.rs` | Replace `.unwrap()` on mutex locks |
| `crates/node/src/validator_pipeline.rs` | Replace `.unwrap()` on privkey access |
| `crates/common/src/block.rs` | Replace `.unwrap()` in genesis timestamp and merkle root |
| `contracts/Staking.sol` | Add `nonReentrant` guards |
| `contracts/Governance.sol` | Add `nonReentrant` guards |
| `crates/validator/src/typosquat.rs` | Load from external dataset instead of hardcoded lists |

---

## Success Criteria for Phase 1

- [ ] All P0 security issues resolved and unit-tested
- [ ] Solidity contracts compile with new `nonReentrant` guards
- [ ] Rust workspace compiles with zero new warnings
- [ ] Testnet with 10+ validators operational for 7 days without critical failures
- [ ] External security audit can meaningfully begin
