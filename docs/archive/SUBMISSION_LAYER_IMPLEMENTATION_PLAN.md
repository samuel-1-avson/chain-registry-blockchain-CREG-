# Implementation Plan: Submission Layer High-Performance & Security Hardening

This roadmap addresses the critical bottlenecks and security enhancements identified in the Submission Layer review. It delivers immediate 10x admission throughput gains via an in-memory L1 stake cache, patches archive decompression vulnerability vectors (anti-zip-bomb / ReDoS guards), and establishes the architecture for direct gRPC stream ingestion.

## User Review Required

> [!IMPORTANT]
> **Phase 1 Execution Scope (Immediate Work):**
> 1. **In-Memory L1 Stake Cache (`NodeState`):** Caches publisher staked balances with a configurable TTL (default: 5 minutes) and on-demand refresh, eliminating synchronous Ethereum JSON-RPC blocking on repeated admissions.
> 2. **Decompression Ratio & Archive Guard (`ml-validator`):** Enforces hard memory, file count (max 10,000 files), and uncompressed expansion ratio ceilings (max 100:1 ratio, 256MB total) during in-memory tarball extraction to prevent zip bombs.
> 3. **Validation & Test Coverage:** Extends unit tests in `package_admission.rs` and `deep_scan.rs` verifying cache behavior, expired stake evictions, and zip-bomb abort semantics.

> [!NOTE]
> All existing fail-closed security invariants (dual Ed25519 + EVM EIP-191 signatures, CR-07 namespace authority proofs, and YARA pre-mempool rules) remain strictly enforced.

---

## Proposed Changes

### Component 1: `chain-registry-node` (Admission Engine & L1 Stake Cache)

#### [MODIFY] [state.rs](file:///f:/project/chain-registry/chain-registry/crates/node/src/state.rs)
- Add `PublisherStakeCache` to `NodeState` storing `HashMap<Address, (alloy::primitives::U256, std::time::Instant)>`.
- Provide helper methods: `get_cached_publisher_stake`, `set_cached_publisher_stake`, and `invalidate_publisher_stake`.

#### [MODIFY] [package_admission.rs](file:///f:/project/chain-registry/chain-registry/crates/node/src/package_admission.rs)
- Update `ensure_publisher_staked`:
  - Check in-memory `publisher_stake_cache` first.
  - If valid and unexpired (TTL $\le 300\text{s}$), return immediately in $<0.1\text{ms}$.
  - If cache miss or expired, perform the RPC `stakedBalance` call, populate the cache, and proceed.
- Add unit tests for `ensure_publisher_staked` testing cache hits, TTL expiry, and zero-balance rejection.

---

### Component 2: `ml-validator` (Pre-Mempool Scanning & Anti-Zip-Bomb Guards)

#### [MODIFY] [deep_scan.rs](file:///f:/project/chain-registry/chain-registry/crates/ml-validator/src/deep_scan.rs)
- Update `extract_source_files`:
  - Enforce `MAX_TOTAL_EXTRACTED_BYTES` (256 MiB limit).
  - Enforce `MAX_SINGLE_FILE_BYTES` (32 MiB limit per file).
  - Enforce `MAX_ARCHIVE_FILE_COUNT` (10,000 files limit).
  - Enforce `MAX_DECOMPRESSION_RATIO` (100:1 relative to compressed archive bytes).
  - Abort extraction with `std::io::Error(InvalidData, "Decompression limit exceeded")` if exceeded.
- Add tests in `deep_scan.rs` verifying clean extraction and bomb rejection.

---

## Verification Plan

### Automated Tests
1. **Node Unit & Admission Tests:**
   ```powershell
   cd f:\project\chain-registry\chain-registry\crates\node
   cargo test package_admission -- --nocapture
   cargo test
   ```
2. **ML-Validator Decompression & Scan Tests:**
   ```powershell
   cd f:\project\chain-registry\chain-registry\crates\ml-validator
   cargo test deep_scan -- --nocapture
   cargo test
   ```
3. **Full Workspace Integrity Check:**
   ```powershell
   cd f:\project\chain-registry\chain-registry
   cargo fmt --all -- --check
   cargo test --locked --workspace --all-targets -j1
   ```

### Manual / Integration Verification
- Verify that repeated submissions from the same staked publisher bypass external RPC calls and execute in sub-millisecond time.
- Verify that a crafted high-compression archive fails gracefully at pre-mempool admission without starving node memory or CPU.
