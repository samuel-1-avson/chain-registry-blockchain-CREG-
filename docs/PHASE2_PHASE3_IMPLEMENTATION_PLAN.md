# Implementation Plan: Phase 2 & Phase 3 Submission Layer Evolution

Architectural design and staged implementation roadmap for **Phase 2 (Performance & Isolation)** and **Phase 3 (Mainnet Security & Cryptographic Provenance)** of the CREG Submission Layer.

---

## User Review Required

> [!IMPORTANT]
> **Phase 2 gRPC Streaming Wire Compatibility**: `StreamPublishPackage` will be added alongside the existing `SubmitPackage` unary gRPC method in `proto/node.proto`, maintaining 100% backward compatibility for existing scripts, CI pipelines, and older CLI binaries.

> [!NOTE]
> **Phase 3 Staking Contract Expansion**: Dynamic risk-tiered staking will introduce an on-chain tier registry in `Staking.sol` allowing governance or registry consensus to categorize namespace tiers (e.g., tier-1 infrastructure vs tier-3 leaf packages) without breaking existing flat-stake publishers.

---

## Open Questions

- **Worker Process Sandboxing Model (Phase 2.2)**: Do you prefer in-process bounded Tokio worker pools (`tokio::task::spawn_blocking` with strict CPU/memory time limits) or isolated out-of-process IPC sidecar daemons (like `creg-scanner-worker`)?
  *(Recommendation: In-process bounded threadpools with cgroup memory limits for Phase 2, evolving to standalone worker daemons in Phase 3).*
- **Hardware Wallet Device Scope (Phase 3.2)**: Should Phase 3 prioritize Ledger Ethereum app signing (EIP-191/EIP-712) or FIDO2/WebAuthn (YubiKey / Passkeys)?
  *(Recommendation: Support both by implementing Ledger EIP-712 first, followed by WebAuthn P-256 assertions).*

---

## Proposed Changes

---

### Phase 2: High-Throughput Ingestion & Compute Isolation

#### 1. Proto & gRPC Streaming Layer (`crates/common`, `crates/node`, `crates/cli`)

- **[MODIFY] [`crates/common/proto/node.proto`](../crates/common/proto/node.proto)**:
  - Add `StreamPublishPackage(stream PublishChunk) returns (SubmitResponse)` to `RegistryService`.
  - Define `PublishChunk` message supporting header metadata in chunk 0 and bounded raw bytes (e.g. 512KB–1MB chunks) in subsequent stream frames.
- **[MODIFY] [`crates/node/src/grpc/server.rs`](../crates/node/src/grpc/server.rs)**:
  - Implement `stream_publish_package`:
    - Reads initial metadata chunk to verify dual signatures, namespace authorization, and stake cache.
    - Streams byte chunks directly into the local IPFS pinner / storage buffer while computing SHA-256 incrementally.
    - Rejects early if byte limit or decompression ratio is exceeded mid-stream.
- **[MODIFY] [`crates/cli/src/publish.rs`](../crates/cli/src/publish.rs)**:
  - Add `--streaming` flag (enabled by default when targeting gRPC endpoints).
  - Slice tarball into async streaming chunks with progress bar updates.

#### 2. Sandboxed Scanner Worker Pool (`crates/ml-validator`, `crates/validator`, `crates/node`)

- **[MODIFY] [`crates/ml-validator/src/lib.rs`](../crates/ml-validator/src/lib.rs)**:
  - Add an asynchronous `ScanWorkerPool` with dedicated worker threads, preventing CPU-bound AST parsing or YARA evaluation from stalling Tokio async I/O or PBFT consensus gossip.
  - Implement per-scan execution timeouts and abort handles.
- **[MODIFY] [`crates/node/src/package_admission.rs`](../crates/node/src/package_admission.rs)**:
  - Offload pre-mempool YARA scans to the dedicated worker pool before pending pool insertion.

---

### Phase 3: Mainnet Security, Provenance & Hardware Keys

#### 1. Dynamic Risk-Tiered Staking (`contracts/`, `crates/node`)

- **[MODIFY] [`contracts/src/Staking.sol`](../contracts/src/Staking.sol)**:
  - Add namespace tier lookup (`tierOfNamespace(bytes32)`).
  - Define minimum stake by tier (e.g. Core: $10.0\text{ }tCREG$, Standard: $3.0\text{ }tCREG$, Leaf: $1.0\text{ }tCREG$).
- **[MODIFY] [`crates/node/src/package_admission.rs`](../crates/node/src/package_admission.rs)**:
  - Query tier minimums in `ensure_publisher_staked` and enforce tiered balance checks against `PublisherStakeCache`.

#### 2. Hardware Security & WebAuthn Signatures (`crates/cli`, `crates/common`)

- **[MODIFY] [`crates/cli/src/publish.rs`](../crates/cli/src/publish.rs)**:
  - Add `--ledger` and `--webauthn` signing options.
  - Integrate hardware wallet transport abstraction for EIP-191 personal signing.
- **[MODIFY] [`crates/common/src/crypto.rs`](../crates/common/src/crypto.rs)**:
  - Add WebAuthn signature envelope verifier for P-256 / secp256k1 credentials.

#### 3. ZK-SNARK Build Provenance Attestation (`crates/zk-validator`, `crates/cli`)

- **[MODIFY] [`crates/zk-validator/src/circuits.rs`](../crates/zk-validator/src/circuits.rs)**:
  - Define `BuildProvenanceCircuit` verifying that `content_hash == H(source_hash, compiler_config, deterministic_build_receipt)`.
- **[MODIFY] [`crates/cli/src/publish.rs`](../crates/cli/src/publish.rs)**:
  - Add `--zk-prove` flag to generate local Groth16 proofs during compilation.
- **[MODIFY] [`crates/node/src/package_admission.rs`](../crates/node/src/package_admission.rs)**:
  - Verify ZK build proofs in sub-millisecond time on the validator node, enabling instant fast-track admission.

---

## Verification Plan

### Automated Test Suites
1. **gRPC Chunk Streaming Tests**:
   - `cargo test -p chain-registry-node --test e2e stream_publish`
   - Test streaming 1MB, 10MB, and 50MB packages with simulated network chunking.
2. **Scanner Worker Isolation Tests**:
   - `cargo test -p ml-validator worker_pool`
   - Test parallel scans under high CPU load without blocking concurrent async channels.
3. **Dynamic Staking Contract Tests**:
   - `forge test --match-contract StakingTest`
   - Verify tier-based minimum balance validations and slashing.
4. **ZK Provenance Tests**:
   - `cargo test -p zk-validator circuits`
   - Verify Groth16 proof generation and verification for package build inputs.
