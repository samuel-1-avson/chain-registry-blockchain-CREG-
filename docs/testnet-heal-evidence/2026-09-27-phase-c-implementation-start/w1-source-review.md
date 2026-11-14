# W1 source review: durable local COMMIT capture

**Updated:** 2026-09-27 02:38 UTC (source review began at 00:43 UTC)  
**Chain:** `creg-phase-c-testnet-2026-09-26-r7`  
**Purpose:** record the first implementation pass for the approved remaining-work plan. This report is source and read-only fleet evidence; it is not a crash-capture result.

## W0 live baseline

The read-only collector ran at 2026-09-27 00:12:51 UTC. All eight real nodes passed readiness, recovery-clear and PBFT-clear checks. Each reported the same h12 tip `a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d`, R7 chain ID and genesis, accepted scanner profile `d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4`, four-validator roster and live peers. The four added GCP hosts were `RUNNING`; the node collector independently confirmed the actual CREG containers were healthy.

Full per-node responses and checks: [W0 fleet readiness](w0-fleet-readiness.json).

After W1a's four consistent validator database copies, a fresh read-only fleet capture completed at 2026-09-27 02:32 UTC. All eight nodes again passed readiness, recovery-clear and PBFT-clear checks at the accepted h12 tip. See [post-W1a fleet readiness](w1a-after-v1-snapshot.json).

## Existing recovery order

1. `PbftSafetyJournal::record_decision` validates the local signing identity, decision payload, signature and conflicting durable locks.
2. The journal writes the decision through RocksDB `write_opt` with `WriteOptions::set_sync(true)` and returns only after the synchronous write succeeds.
3. The proposer then applies the local COMMIT to the in-memory PBFT engine, prepares its gossip command, and only after leaving the state lock broadcasts the vote. A local quorum can then insert the finalized block.
4. Startup loads and validates the journal, restores the matching durable prepared block first, then inserts the recovered local COMMIT into the PBFT engine. Invalid parent, height, block, identity, signature or missing prepared state fails closed.
5. Startup completes this recovery before it starts P2P, HTTP serving, or sync workers. Therefore a recovered-local-COMMIT log emitted in this section is direct evidence that startup replay preceded peer reconciliation.

Relevant code: `chain-registry/crates/node/src/pbft_safety.rs`, `block_producer.rs`, and `main.rs`.

## R22 capture instrumentation

The candidate source now emits `pbft_local_commit_durable` immediately after the synchronous journal call returns. Startup replay emits `pbft_local_commit_recovered` with the validator, chain, genesis, height, parent and block hash.

Only builds with `phase-c-real-fleet` contain the pause. It remains inactive unless all six `CREG_PHASE_C_COMMIT_CAPTURE_*` values are supplied: `NODE_ID`, `CHAIN_ID`, `GENESIS_HASH`, `HEIGHT`, `PACKAGE_CANONICAL`, and `PAUSE_MS`. Startup requires a testnet validator whose configured node and signed chain identity match the arm. The pause matches the exact block height and a real `Revoke` transaction for the named package. The maximum is 30 seconds; it occurs after durable journal persistence and before in-memory COMMIT application or broadcast. With no arm set, the candidate does not pause.

The pause holds V4's node-state write lock during the bounded window, so V4 API and worker state access will wait. The planned drill must keep the other three validators online, SIGKILL only the V4 container after the pause marker, and restart the exact container using its existing data volume. The recovery log must precede P2P/sync startup; the captured V4 tip must show whether replay occurred before peer sync.

No private signing material is logged or added to the evidence. The runtime arm is compiled only in the Phase C candidate; standard node builds do not include the pause implementation.

## Build and static R22 image review

The native Windows `cargo check` stopped before project compilation because `librocksdb-sys` could not locate `libclang.dll`. The pinned Linux Docker build completed successfully as `creg-node:phase-c-secure-r22-commit-capture`, image ID `sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3`. The frozen manifest contains 731 files; a fresh hash comparison found zero mismatches. The R22 node binary SHA-256 is `c7ef1d3e8bb9b34e2e0e4e255bf47f751ecaada4d0617f9ac5a4dbedb262a5b1`, and the CLI binary SHA-256 is `b53d897734ee84523e00b24ce1e248969bc38f1b096c73c9d8942e916e894ad5`.

Static image inspection confirmed Linux/amd64, non-root runtime user `creg`, `/data` volume, and `/app/creg-node` entrypoint. The image does not contain the local `testnet` directory; `.dockerignore` excludes local environment files, credentials, key/certificate files, generated fleet environments and local runtime state. The final node binary contains the accepted R7 chain-spec signer key, while the Dockerfile default signer key is absent. The final image does not include `creg_recovery_inspect`; W1a completed its inspection with the preserved R21 image before rollout. Full values are in [R22 image review](image-review.json).

The build completed at 2026-09-27 01:25 UTC. The R22 image remains local and undeployed. No R22 candidate container was started, no tests were run, and no real-host acceptance is claimed from the static artifact review. Build logs, status, and the source manifest are preserved beside this review.

## W1 pass criteria still open

- [x] R22 node and CLI build completes; record image ID, source hashes and binary checksums.
- [x] Inspect final image and build context for accidental local credentials and check the embedded R7 spec signer.
- [ ] Review exact V4 arm fields and target height after W2 identifies the real package and publication height.
- [ ] Roll the immutable R22 artifact sequentially through validators with three other validators online, preserving each node's data and signer journal; then align observers.
- [ ] Confirm the new durable and startup-replay markers from a normal real commit/restart before the controlled W3 crash capture.

## W1a retained-work gate

W1a passed after a read-only inspection of consistent database copies from all four validators and a canonical reconciliation of all 25 records. All are terminal duplicates of already-committed R7 publications or revocations; active work and unresolved records are zero. None were replayed or deleted. The full disposition and per-node evidence are in [W1a review](w1a-dead-letter-review.md). V3's original snapshot does not have a contemporaneous metadata JSON file; the database, diagnostics, hash manifest and post-copy fleet evidence are preserved.

Public cutover and chain reset were not performed. The four additional GCP VMs remain running as required testnet nodes.
