# Implementation Plan: Layer 2 PBFT Consensus Hardening

## Problem & Background

In **Layer 2 Block Production & State Machine Finality**, two key scalability and security gates exist:
1. **Signature Overhead**: Each PBFT block currently stores individual Ed25519 signatures from all $2f + 1$ voting validators in the block header. In large validator sets (e.g. 100+ to 10,000+ nodes), storing individual signatures inflates block header size ($O(N)$) and consumes network gossip bandwidth.
2. **Equivocation Detection & Automated Slashing**: Byzantine proposers or validators attempting double-signing (voting on conflicting blocks at the same height) must be penalized. While `zk-validator` and `contracts/ZKSlashingVerifier.sol` implement double-sign verification, a dedicated consensus bridge is required to package double-sign evidence automatically and prepare on-chain slashing transactions.
3. **Pipelined Fast-Path Finality**: Sequential 3-phase PBFT introduces latency when waiting for full round transitions. Adding pipelined fast-path transitions allows validators to pipelined commit and prepare the next block height concurrently.

---

## Proposed Changes

### 1. `crates/consensus` (BN254/BLS Multi-Signature Aggregation)

#### [NEW] [`crates/consensus/src/bls_aggregate.rs`](../crates/consensus/src/bls_aggregate.rs)
- Implement `BlsAggregateSignature` and `BlsKeySet` using pairing-friendly BN254 / BLS cryptography (`ark-bn254`, `ark-ec`, `ark-ff`).
- Provide:
  - `aggregate_signatures(signatures: &[Signature]) -> Result<BlsAggregateSignature>`
  - `verify_aggregate_signature(agg_sig: &BlsAggregateSignature, message: &[u8], public_keys: &[PublicKey], signer_bitmask: &[u8]) -> Result<bool>`
  - Header compression: Compresses $M$ individual signatures into a single 64-byte/128-byte aggregate point with a bitmask.

#### [MODIFY] [`crates/consensus/src/lib.rs`](../crates/consensus/src/lib.rs)
- Expose `pub mod bls_aggregate;` and `pub mod slashing_relay;`.

---

### 2. `crates/consensus` & `crates/node` (Equivocation Slashing Relay Bridge)

#### [NEW] [`crates/consensus/src/slashing_relay.rs`](../crates/consensus/src/slashing_relay.rs)
- Implement `EquivocationDetector`:
  - Tracks validator votes across heights, views, and phases.
  - Detects conflicting signatures: $\text{Sig}_1(H, V, B_1)$ and $\text{Sig}_2(H, V, B_2)$ where $B_1 \neq B_2$.
  - Generates verifiable `EquivocationSlashingPayload` compatible with `contracts/ZKSlashingVerifier.sol` and `contracts/Staking.sol`.

---

### 3. `crates/consensus` (Pipelined PBFT Fast-Path Execution)

#### [MODIFY] [`crates/consensus/src/pbft.rs`](../crates/consensus/src/pbft.rs)
- Add `try_fast_commit`: When $\ge \lfloor 2n/3 \rfloor + 1$ Prepare signatures are gathered, transitions immediately to `CommitQuorum` and exports pipelined state indicators.
- Provide unit tests verifying aggregate signature compression, double-sign equivocation detection, and pipelined fast-path finality.

---

## Verification Plan

### Automated Tests
1. **`consensus` crate test suite**:
   ```bash
   cargo test -p consensus
   ```
   Verify BLS/BN254 aggregate signature generation and verification, bitmask verification, and equivocation detector.
2. **`chain-registry-node` test suite**:
   ```bash
   cargo test -p chain-registry-node --lib
   ```
   Verify PBFT engine integration with pipelined fast commits and block assembly.
3. **Workspace-wide formatting & build check**:
   ```bash
   cargo fmt --all -- --check
   ```
