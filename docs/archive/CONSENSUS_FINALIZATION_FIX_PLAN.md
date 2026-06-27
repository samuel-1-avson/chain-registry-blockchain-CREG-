# Consensus Finalization Fix Plan

**Status:** Draft (2026-06-14)  
**Context:** Public testnet package `npm:@halldean/creg-smoke@1.0.20260614062150` was admitted to the observer pending pool, reached **2/2 evidence votes** (`/v1/consensus/state` → `quorum-reached`), but **`tip_height` stayed 0** and the package remained **`pending` / UNVERIFIED** for 4+ minutes. Publisher stake, signing, and IPFS were all correct after remediation.

This document is the implementation plan to fix chain finalization, harden the publish path, and prevent recurrence.

---

## Executive summary

| Layer | Symptom | Likely root cause |
|-------|---------|-------------------|
| **UX / API** | `quorum-reached` looks like “done” | Evidence votes (gossip) ≠ PBFT block commit ≠ chain `verified` |
| **Block production** | `tip_height: 0`, `pending_tx_count: 1` | Finalized transactions never become committed blocks |
| **Code (high confidence)** | Validators vote but chain idle | `block_producer` **drops** drained txs when VRF proposer gate fails |
| **Fleet** | Observer shows votes, no blocks | Validators may not commit; observer syncs from peers also at height 0 |
| **Publish path** | HTTP 502 (~30s) | GCP LB timeout = admission IPFS timeout; tarball only on local IPFS |
| **Future liveness** | No anti-censorship | `forced_inclusion` exists but is **not wired** in `crates/node` |

**Top 5 must-do fixes (in order):**

1. **Stop losing finalized transactions** in `block_producer.rs` when proposer gate fails.
2. **Add integration test:** 2-validator fleet → submit → `tip_height ≥ 1` → package `verified` within SLA.
3. **Pin to fleet IPFS on publish** (`CREG_IPFS_URL` default / doctor check) so admission never hangs.
4. **Raise GCP LB backend timeout** above worst-case admission (or fail fast + async admit).
5. **Observability + alerts** on `tip_height` stall, finalized-tx queue depth, PBFT round phase age.

---

## Architecture (current)

```
Publisher → POST /v1/publisher/packages (observer read pool)
         → pending_pool (observer)
         → gossip creg/v1/submissions
              ↓
Validators (CREG_IS_VALIDATOR=true)
         → pending_pool insert (after YARA pre-mempool gate)
         → validator_pipeline::process_package
              → sandbox + local vote
              → gossip creg/v1/votes
              → wait evidence quorum (aggregate_evidence_votes)
              → mpsc finalized_tx channel
         → block_producer (every CREG_BLOCK_INTERVAL s)
              → drain finalized_tx
              → VRF proposer selection
              → PBFT PrePrepare → Prepare → Commit (gossip creg/v1/blocks)
              → chain.insert_block
              ↓
Observer (CREG_IS_VALIDATOR=false)
         → does NOT run validator_pipeline (by design)
         → sync.rs polls peer /v1/chain/stats + fetches blocks
         → GET /v1/packages/:id → pending until chain record exists
```

**Critical design points:**

- `GET /v1/consensus/state` reflects **`package_rounds`** (gossiped evidence votes), not PBFT block rounds (`crates/node/src/api.rs`).
- Observer **intentionally** keeps packages in `pending_pool` until chain sync provides a `verified` record (`validator_pipeline.rs` L38–42).
- PBFT quorum for **n=2** validators is **2/2** (`quorum_threshold` in `crates/consensus/src/pbft.rs` L457–462). `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM` only relaxes **n=3** to 2/3.
- `forced_inclusion.rs` L5–8: **not wired** into node; no runtime censorship recovery.

---

## Root cause analysis (ranked)

### RC-1 — Finalized txs dropped when proposer gate fails (**P0 bug**)

**File:** `crates/node/src/block_producer.rs`

Every tick:

```rust
let txs = finalized_tx::drain(&rx).await;  // L61 — irreversible drain
if txs.is_empty() { continue; }
match produce_block(..., txs, ...).await {   // L67
    Ok(block) => { /* broadcast PbftPrePrepare */ }
    Err(e) => tracing::error!(...),         // L101 — txs are NOT re-queued
}
```

`produce_block` returns `Err` when the node is not the VRF rank-0 proposer (L231–237) or when VRF ranking is empty. **All validators run `block_producer`**, but only one may propose per epoch. Non-proposers (or a proposer that loses the race on tick alignment) **drain and discard** finalized transactions.

**Why this matches live symptoms:**

- Evidence votes complete (validators finished `process_package` through vote gossip).
- `pending_tx_count: 1` on observer (chain never got a publish tx in a block).
- `tip_height: 0` (no successful `insert_block` after genesis).
- `quorum-reached` visible on observer (votes gossiped; does not require block commit).

**Fix direction:** Never drain unless this node will propose, **or** move drained txs into a durable `PendingBlockTx` buffer that only the current proposer consumes.

---

### RC-2 — Evidence quorum ≠ chain finalization (**P0 confusion / observability**)

**Files:** `crates/node/src/api.rs` (consensus state), `crates/node/src/validator_pipeline.rs`, CLI `creg status`

Operators and publishers interpret `quorum-reached` as “verified.” In reality:

| Stage | Meaning | API signal |
|-------|---------|------------|
| Evidence votes | Validator sandbox approvals | `/v1/consensus/state` → `quorum-reached` |
| Finalized tx | Tx enqueued for block | No public metric today |
| PBFT block | 2/2 prepare+commit | No public metric today |
| Chain record | Package durable | `/v1/packages/...` → `verified`, `tip_height` increases |

**Fix direction:** Split public status; expose pipeline stage in API and CLI.

---

### RC-3 — Observer / validator fleet split (**P1 fleet**)

**Files:** `testnet/docker-compose.validator-fleet.yml`, `testnet/start-observer-pool-gcp.sh`, `testnet/gcp/hotfix-caddy-upstream.sh`

- Public `api.testnet.cregnet.dev` → observer read pool (Caddy → `CREG_OBSERVER_API_UPSTREAM`).
- Validators run on separate VM(s); only **2** of **3** fleet containers are validators (`core-1`, `validator-2`; `observer-1` is non-validator).
- Submissions must reach validators via **P2P gossip** (`creg/v1/submissions`). Vote gossip clearly works (observer sees 2/2). Block PBFT requires **validator-validator** gossip on `creg/v1/blocks`.

**Risk:** P2P mesh or VRF proof exchange incomplete → proposer cannot produce or PBFT stalls.

**Fix direction:** Fleet smoke test proving `tip_height` advances after submit; document required `CREG_PEERS` / `CREG_P2P_SEEDS` between observer and validators.

---

### RC-4 — Publish / admission timeouts (**P1 infra + product**)

**Files:** `testnet/gcp/setup-gcp-public-lb.ps1` (L123 `--timeout=30s`), `crates/node/src/admission_scan.rs` (`CREG_PRE_MEMPOOL_IPFS_TIMEOUT_SECS` default 30s)

- Node admission fetches tarball from IPFS during `POST /v1/publisher/packages`.
- GCP LB backend timeout is **30s** → HTML 502 even when node eventually admits (first submit may succeed with 502 to client).
- Publishing with `CREG_IPFS_URL=http://127.0.0.1:5001` pins only locally; fleet IPFS never has CID → admission hangs until timeout.

**Fix direction:** Pin to fleet IPFS in CLI publish path; raise LB timeout to 60–120s; add doctor check.

---

### RC-5 — `forced_inclusion` not wired (**P2 liveness**)

**File:** `crates/consensus/src/forced_inclusion.rs` L5–8

No integration in `block_producer` or `validator_pipeline`. Stuck finalized txs have no protocol-level recovery.

---

## Implementation plan

### Phase 0 — Confirm diagnosis on fleet (ops, 1 day)

| # | Action | Owner | Verify |
|---|--------|-------|--------|
| 0.1 | SSH validator VM; `docker logs creg-fleet-node1 \| grep -E "Block production failed\|FINALISED\|→ VERIFIED"` | Ops | Find dropped-tx errors |
| 0.2 | `curl` validator internal APIs `:28180`/`:28181` `/v1/public/chain/stats` | Ops | If validators also `tip_height: 0`, bug is on validators not observer sync |
| 0.3 | `curl` `/v1/consensus/state` on validator vs observer | Ops | Confirm evidence quorum on both |
| 0.4 | Capture pending pool on validator: operator endpoint `/v1/pending` with `CREG_OPERATOR_API_KEY` | Ops | Package should be absent on validators after pipeline cleanup |

**Exit criteria:** Logs confirm RC-1 (proposer gate error after pipeline success) or identify alternate PBFT failure.

---

### Phase 1 — P0 code fixes (1–3 days)

#### 1.1 — Durable finalized-tx buffer (RC-1)

| Item | Detail |
|------|--------|
| **Files** | `crates/node/src/finalized_tx.rs`, `crates/node/src/block_producer.rs`, `crates/node/src/lib.rs` (NodeState) |
| **Change** | Replace blind `drain` with: (a) `try_recv` peek + proposer check, **or** (b) append drained txs to `NodeState.pending_block_txs: Vec<Transaction>` and only `produce_block` consumes when `is_current_proposer()` |
| **On `produce_block` Err** | Re-insert txs at front of buffer (never drop) |
| **Risk** | Memory bound — cap buffer size; emit metric when cap hit |
| **Verify** | Unit test: non-proposer tick does not reduce queue length; proposer tick builds block with queued txs |

#### 1.2 — Proposer-only drain (alternative/complement)

| Item | Detail |
|------|--------|
| **Files** | `block_producer.rs` L43–65 |
| **Change** | Before drain, compute VRF ranking for epoch; **skip tick** if node is not rank-0 and `allowed_fallback_rank` does not apply |
| **Verify** | 2-node local compose: submit package → `tip_height == 1` within 30s |

#### 1.3 — Pipeline stage in API + CLI (RC-2)

| Item | Detail |
|------|--------|
| **Files** | `crates/node/src/api.rs` (`get_package`, new `/v1/public/packages/:canonical/stages`), `crates/cli/src/info.rs` |
| **Change** | Return `evidence_phase`, `in_finalized_queue`, `last_block_attempt`, `chain_status` separately |
| **CLI** | `creg status` prints: `evidence: quorum-reached | block: awaiting-proposer | chain: pending` |
| **Verify** | API contract test; CLI snapshot test |

#### 1.4 — CLI publish pins to fleet IPFS (RC-4)

| Item | Detail |
|------|--------|
| **Files** | `crates/cli/src/publish.rs`, `crates/cli/src/doctor.rs`, `docs/PUBLIC_TESTNET_QUICKSTART.md` |
| **Change** | When `CREG_NODE_URL` is remote testnet, doctor warns if `CREG_IPFS_URL` is localhost; optional `creg publish --ipfs-url` flag |
| **Post-upload** | `POST {ipfs}/api/v0/pin/add?arg={cid}` (best-effort) |
| **Verify** | Publish smoke: `cat` on `ipfs.testnet.cregnet.dev` returns 200 before submit |

---

### Phase 2 — PBFT / sync hardening (3–5 days)

#### 2.1 — PBFT observability

| Item | Detail |
|------|--------|
| **Files** | `crates/node/src/api.rs`, `crates/consensus/src/pbft.rs` |
| **Change** | `GET /v1/consensus/pbft` — active rounds, phase, prepare/commit counts, age |
| **Metrics** | Prometheus: `creg_pbft_round_phase`, `creg_pbft_round_age_seconds` |

#### 2.2 — Sync improvements for observer

| Item | Detail |
|------|--------|
| **Files** | `crates/node/src/sync.rs`, `crates/node/src/api.rs` |
| **Change** | Log peer heights on stall; expose `sync.lag_blocks` in `/v1/health` |
| **Verify** | Observer behind validator by 1 block → sync catches up in <20s |

#### 2.3 — Wire `forced_inclusion` (RC-5)

| Item | Detail |
|------|--------|
| **Files** | `crates/consensus/src/forced_inclusion.rs`, `block_producer.rs`, `validator_pipeline.rs` |
| **Change** | On finalized tx enqueue → `tracker.submit()`; on block commit → `mark_included()`; proposer must include `forced_transactions()` or face griefing flag (log first, slash later) |
| **Verify** | Test: artificially skip tx for N blocks → forced set non-empty → next block includes it |

#### 2.4 — Small-cluster PBFT docs + fleet env

| Item | Detail |
|------|--------|
| **Files** | `testnet/docker-compose.validator-fleet.yml`, `testnet/sepolia-3node.env.example`, `.env.example` |
| **Change** | Document that 2-validator testnet requires both nodes healthy; for local dev set `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=true` only when `CREG_TESTNET=true` (already gated in `config.rs` L470) |
| **Note** | For n=2, flag does not change quorum (still 2/2) — do not rely on flag for 2-node fix |

---

### Phase 3 — Infra / fleet (2–3 days, parallel)

#### 3.1 — GCP LB timeout (RC-4)

| Item | Detail |
|------|--------|
| **File** | `testnet/gcp/setup-gcp-public-lb.ps1` L123 |
| **Change** | `--timeout=120s` (or 90s); document interaction with `CREG_PRE_MEMPOOL_IPFS_TIMEOUT_SECS` |
| **Rollout** | `gcloud compute backend-services update ... --timeout=120s` on live backend |
| **Verify** | Submit large package; client receives 202 within timeout |

#### 3.2 — Admission fail-fast option

| Item | Detail |
|------|--------|
| **File** | `admission_scan.rs`, `package_admission.rs` |
| **Change** | Optional `CREG_ADMISSION_ASYNC=true`: return 202 immediately, run IPFS+YARA in background task |
| **Benefit** | Removes LB coupling entirely |

#### 3.3 — Fleet smoke gate in CI

| Item | Detail |
|------|--------|
| **Files** | `testnet/hub-explorer-smoke.ps1` or new `testnet/consensus-finalization-smoke.ps1` |
| **Change** | After publish: poll until `tip_height > 0` OR `status == verified` with 5 min timeout; fail CI if stall |
| **Wire** | GitHub Actions `rust-ci.yml` or nightly workflow |

#### 3.4 — gRPC ingress documentation

| Item | Detail |
|------|--------|
| **Files** | `docs/PUBLIC_TESTNET_QUICKSTART.md`, `crates/cli/src/publish.rs` |
| **Change** | Document that `:50051` is **not** public; HTTP submit is supported; remove inferred gRPC default for remote hosts |

---

### Phase 4 — Observability & incident response (2 days)

| Metric / alert | Threshold | Action |
|----------------|-----------|--------|
| `creg_chain_tip_height` stale | No increase 10 min | Page ops (SEV-2 per `INCIDENT_RESPONSE_RUNBOOK.md`) |
| `creg_finalized_tx_queue_depth` | >0 for 2 min while tip stale | RC-1 regression |
| `creg_pending_pool_in_progress_stuck` | >5 min | `pending_pool.rs` retry |
| `creg_evidence_quorum_without_block` | quorum age >3 min, tip=0 | Finalization stall |
| GCP LB 502 rate on `/v1/publisher/packages` | >5% | Timeout / IPFS |

**Files:** `observability/prometheus.yml`, `crates/node/src/metrics.rs` (add if missing), `docs/INCIDENT_RESPONSE_RUNBOOK.md` (add “finalization stall” playbook).

---

## Test plan

### Local reproduction (required before fleet deploy)

```powershell
cd chain-registry/testnet
# Uses CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=true for 3-node local compose
docker compose -f docker-compose.local-testnet.yml up -d
# Publish npm tarball, stake publisher on Sepolia local fork if needed
# Assert within 60s:
curl -s http://localhost:28180/v1/public/chain/stats | jq .tip_height   # >= 1
creg status npm:@scope/pkg@1.0.0 --node-url http://localhost:28180      # verified
```

### New automated tests

| Test | Location | Asserts |
|------|----------|---------|
| `finalized_tx_not_dropped_when_not_proposer` | `crates/node/tests/` | Queue length preserved |
| `two_validator_publish_e2e` | `crates/node/tests/e2e.rs` | Evidence → block → verified |
| `pbft_two_of_two_commits` | `crates/node/tests/integration.rs` | Extend for publish tx in block |
| `admission_ipfs_timeout_json_error` | `crates/node/tests/` | 503 JSON, not LB HTML |

### Fleet verification (post-deploy)

```bash
# 1. Submit smoke package (pinned on testnet IPFS)
# 2. Poll validators directly (internal IPs)
curl -s http://VALIDATOR:28180/v1/public/chain/stats
curl -s http://VALIDATOR:28181/v1/public/chain/stats
# 3. Public observer
curl -s https://api.testnet.cregnet.dev/v1/public/chain/stats
creg status npm:@halldean/creg-smoke@1.0.20260614062150
```

**SLA target (testnet):** pending → verified in **< 2 minutes** for unshielded npm smoke packages with 2/2 validators online.

---

## Rollout sequence (unblock current package)

1. **Deploy Phase 1.1** to validator fleet images (`creg-fleet-node1`, `creg-fleet-node2`).
2. **Restart validators** (not observer first): `testnet/gcp/deploy-validator-fleet.ps1` or compose restart.
3. **Re-trigger finalization** for stuck package:
   - Option A: If validators still have package in pending pool, wait for pipeline retry (`pending_pool` stuck >5 min resets `in_progress`).
   - Option B: Re-submit same content hash → expect **409** or idempotent accept; pipeline re-runs.
   - Option C: Operator inject via `/v1/pending` inspect + manual pipeline kick (if tooling added).
4. **Confirm** `tip_height ≥ 1` on validators, then observer sync catches up.
5. **Deploy LB timeout** (Phase 3.1) independent of code release.
6. **Pin existing CID** on testnet IPFS if re-submitting: `QmYHFeeZCYxQFWhPxxJLtAvXos12XZyG7jf73J4Bja8iFs`.

---

## Open questions

1. **Did validators log `Block production failed: ... not its turn`** for the smoke package? (Confirms RC-1.)
2. **Is `pending_tx_count: 1` on observer counting mempool publishes not yet in a block?** Map to `chain/stats` implementation.
3. **Should observers run `block_producer` at all?** Today they drain an empty channel; harmless but confusing.
4. **Minimum validator count for production:** 2/2 PBFT is brittle — recommend 3 validators + `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=true` for 2/3 on testnet.
5. **Async admission:** product decision on 202-fast vs synchronous YARA gate.

---

## Related documents

- `docs/PUBLIC_TESTNET_QUICKSTART.md` — publisher flow
- `docs/TESTNET_PHASE_SCOPE.md` — observer vs validator roles
- `docs/INCIDENT_RESPONSE_RUNBOOK.md` — SEV-2 consensus halt
- `testnet/SOAK_TEST.md` — chain stall procedures
- `docs/WALLET_KEY_DERIVATION.md` — Ed25519 vs EOA staking keys

---

## Appendix — Key code references

| Component | Path | Notes |
|-----------|------|-------|
| Observer skips pipeline | `validator_pipeline.rs` L38–42 | By design |
| Finalized tx drain | `block_producer.rs` L61–65 | Drops on error |
| Proposer gate | `block_producer.rs` L217–237 | Non-proposer bails |
| PBFT quorum n=2 | `pbft.rs` L457–462 | Needs 2/2 |
| Evidence quorum API | `api.rs` L2560–2567 | `quorum-reached` label |
| Package pending response | `api.rs` L1305–1355 | Until chain record |
| P2P submission ingest | `p2p.rs` L414–437 | YARA gate on gossip |
| Chain sync | `sync.rs` L47–73 | HTTP peer polling |
| Forced inclusion unwired | `forced_inclusion.rs` L5–8 | Aspirational |
| LB 30s timeout | `setup-gcp-public-lb.ps1` L123 | 502 source |
