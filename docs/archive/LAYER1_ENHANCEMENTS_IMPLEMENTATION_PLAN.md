# Implementation Plan: Layer 1 Parallel Analysis & Sandbox Enhancements

## Problem & Background

Following our architectural audit of **Layer 1: Parallel Sandbox & Static Analysis / Analysis and Evidence Consensus Layer**, we are addressing three key operational bottlenecks and determinism improvements:

1. **Durable Sandbox Result Persistence**: `validator::sandbox::CANONICAL_STORE` was an in-memory-only map. When a node restarts or a newly assigned validator joins the committee, previous version sandbox execution profiles are missing, causing version diff checks (`DF005–DF007`) to be skipped. We will add durable, thread-safe on-disk persistence so that all validators have identical baseline access across restarts.
2. **Concurrency-Bounded Pipeline Semaphore**: `validator_pipeline.rs` previously spawned an unbounded number of concurrent `tokio::spawn` validation tasks upon receiving a batch of pending packages. Under burst workloads, this risks CPU core saturation and out-of-memory crashes. We will introduce a configurable `tokio::sync::Semaphore` (defaulting to $2 \times \text{CPU cores}$) to queue parallel scans gracefully.
3. **Deterministic Wasmtime Fuel Limits**: WebAssembly execution in `wasm-sandbox` previously relied primarily on host wall-clock timeouts. We will enable Wasmtime fuel consumption (`consume_fuel(true)` and `Store::set_fuel`), enforcing a platform-independent, deterministic instruction budget that prevents infinite loops regardless of host CPU speed variations.

---

## User Review Required

> [!NOTE]
> All changes maintain 100% backward compatibility with existing network state and serialized protobuf schemas.
> The disk persistence for sandbox results is stored under the node's configured `data_dir` (`data_dir/sandbox_history/`) or local fallback directory.

---

## Proposed Changes

### 1. `crates/validator` & `crates/node` (Durable Sandbox Result Persistence)

#### [MODIFY] [`crates/validator/src/sandbox.rs`](../crates/validator/src/sandbox.rs)
- Add disk persistence helpers: `store_result_to_disk` and `get_result_from_disk`.
- Update `store_result(canonical, result)` to save both into the in-memory cache and into the persistent storage directory (configurable via `CREG_SANDBOX_STORE_DIR` or `CREG_DATA_DIR/sandbox_history`).
- Update `get_result(canonical)` to check in-memory cache first, then fall back to disk deserialization if missing.

#### [MODIFY] [`crates/node/src/validator_pipeline.rs`](../crates/node/src/validator_pipeline.rs)
- Ensure the pipeline passes the node's configured `data_dir` to populate sandbox results durably upon completing package validation.

---

### 2. `crates/node` (Concurrency-Bounded Pipeline Semaphore)

#### [MODIFY] [`crates/node/src/validator_pipeline.rs`](../crates/node/src/validator_pipeline.rs)
- Introduce a static `LazyLock<Arc<tokio::sync::Semaphore>>` initialized via `CREG_MAX_CONCURRENT_VALIDATIONS` (default: $\text{CPU cores} \times 2$, minimum 4).
- In `process_package`, acquire an execution permit before launching CPU-heavy sandboxing and AST parsing, automatically releasing it upon task completion or error.

---

### 3. `crates/wasm-sandbox` (Deterministic Wasmtime Fuel Limits)

#### [MODIFY] [`crates/wasm-sandbox/src/limits.rs`](../crates/wasm-sandbox/src/limits.rs)
- Add `pub max_fuel: u64` to `ResourceLimits`.
- Set defaults: `500_000_000` fuel units for standard, `100_000_000` for strict, and `2_000_000_000` for relaxed.
- Add builder method `.with_fuel(fuel: u64)`.

#### [MODIFY] [`crates/wasm-sandbox/src/lib.rs`](../crates/wasm-sandbox/src/lib.rs)
- In `WasmSandbox::new`, configure `engine_config.consume_fuel(true)`.
- When initializing `Store<SandboxState>`, call `store.set_fuel(self.config.limits.max_fuel)`.
- If Wasm execution traps due to out-of-fuel, map the error cleanly to `SandboxError::ResourceLimitExceeded("WASM execution exceeded fuel limit")`.
- Add unit tests verifying that fuel-limited infinite loops terminate deterministically.

---

## Verification Plan

### Automated Tests
1. **`wasm-sandbox` tests**:
   ```bash
   cargo test -p wasm-sandbox
   ```
   Verify fuel consumption limits, strict/relaxed configs, and memory bounds.
2. **`validator` tests**:
   ```bash
   cargo test -p validator
   ```
   Verify durable sandbox storage, disk roundtrip, and diff detection across mock restarts.
3. **`chain-registry-node` tests**:
   ```bash
   cargo test -p chain-registry-node --lib
   ```
   Verify pipeline semaphore behavior, consensus vote gossip, and mempool validation under concurrent batches.
4. **Workspace formatting & full workspace tests**:
   ```bash
   cargo fmt --all -- --check
   ```
