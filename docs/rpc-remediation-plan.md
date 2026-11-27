# CREG RPC: findings and remediation plan

Written 2026-11-27. Scope: the native CREG JSON-RPC (`creg_*`), the Sepolia `eth_*` gateways, and the node's L1 transport (`common::l1_rpc`).

**Evidence key.** *Live* = observed on the public hosts on 2026-11-27 by read-only probes. *Code* = read in the repo. *Inferred* = follows from code or config but not observed. I could not run the Rust tests or reach the VM, so nothing here is runtime-verified on the VM.

Related: [RPC-ALLOWLIST-REDEPLOY.md](../chain-registry/testnet/RPC-ALLOWLIST-REDEPLOY.md) (the step-by-step for Phase 1), ADR-019 (ECA-38, Inc 6).

---

## 1. Findings

| ID | Sev | Area | Finding | Evidence | Phase |
|---|---|---|---|---|---|
| R-01 | High | Edge | The allowlist is not enforced on `api…/sepolia`, `rpc…/sepolia`, `explorer…/rpc` or `faucet…/rpc`. `eth_getLogs`, `web3_clientVersion`, batches of any size and 70 KiB bodies all succeed, and `GET explorer…/rpc` returns 200 instead of 405. Caddy proxies all four straight to the external pool (`via: 2.0 Caddy`), so the explorer container is not in the path at all. | Live (Phase 0 probe) | 1 |
| R-02 | High | Process | Deployed Caddy and explorer image are older than the repo. Deploy paths overlap (unified, cloud-edge, fleet, hybrid). `deploy-unified-stack.ps1` uploads only `Caddyfile.unified` (not `sepolia-public-rpc.caddy`), recreates a stack that has diverged, and its `-TearDownRedundantResources` flag deletes `creg-sepolia-geth-vm`. Explorer digests are pinned in several compose files, and the `VITE_*` build args for the deployed image are not recorded. | Code | 0, 6 |
| R-03 | High | Native RPC | Read-only `creg_*` calls use the "mutating" rate bucket: 30 requests per 60 s per IP. Live, request 31 in a minute got 429. The 429 body (`{"error":"Rate limit exceeded"}`) is not a JSON-RPC error. | Live + code (`rate_limit.rs:223`, `:67`) | 2 |
| R-04 | High | Edge | There is no per-client limiting at the node. `CREG_TRUSTED_PROXIES` is set in no compose file, env file or script, so the node keys every request on its TCP peer (the Caddy or explorer container). All users share one 30/min `/rpc` bucket and one 3000/min `/v1` bucket per entry path, so one client sending 30 calls a minute blocks native RPC for everyone. The explorer's nginx realip path was never exercised (ADR-019). | Code + config (VM unverified) | 2, 4 |
| R-05 | High | L1 config | The observer env file sets `CREG_ETH_RPC` to one third-party host with no fallbacks, so quorum reduces to trusting one vendor. The external validator script defaults to the same vendor. All four public Sepolia surfaces go to one load-balanced pool behind Cloudflare that answers as Geth v1.17.9 or reth v2.11.0 from call to call, which looks like a public provider rather than a CREG-run node. This conflicts with the self-hosting policy. | Code + Live (VM unverified) | 3 |
| R-06 | Med | Native RPC | `creg_health` has no tip time, lag, peer count or pending work. Blocks are produced only when finalized work exists (`metrics.rs:70`), so an old tip (64 minutes at probe time) usually means idle, not stalled, and a client cannot tell the two apart without the pending-work count. (Correction: `finalized: true` is by design, because every stored block passed PBFT; the REST endpoints say the same.) | Live + code | 2 |
| R-07 | Med | Native RPC | No per-method counters, latency or error metrics for the RPC. | Code | 2 |
| R-08 | Med | Native RPC | Protocol gaps: no batch (an array body gives `-32700` with a misleading message); the body is parsed as a positional struct, so `["2.0","creg_chainId"]` is accepted; notifications get a response with `id:null`; a missing `jsonrpc` is accepted. | Live | 2 |
| R-09 | Med | Edge | Browser pages on other origins cannot call `/rpc` or `/sepolia`. Caddy answers `OPTIONS` and `GET` with a static descriptor, so no CORS preflight reaches the node (`OPTIONS /rpc` returned 200 with no `Access-Control-Allow-Origin`). The node's own CORS list is empty by default. | Live + code | 2 |
| R-10 | Med | Availability | Native `/rpc` has one upstream (`creg-phase-c-r7-o1`, `lb_policy first`), and the explorer gateway has one Sepolia upstream. Either one down means the RPC is down. | Code | 4 |
| R-11 | Med | L1 transport | Quorum reads use `join_all`, so latency is that of the slowest endpoint (up to the 15 s timeout) even after a majority already agrees. | Code (`l1_rpc.rs:183`) | 3 |
| R-12 | Med | L1 transport | Failover and primary-only calls keep no endpoint health. A dead primary costs up to a 5 s connect timeout on every call. | Code | 3 |
| R-13 | Med | Edge | The legacy nginx configs `testnet/nginx/explorer-{bootstrap,fleet,hybrid}` still proxy `/rpc` without the allowlist (ADR-019 gap 2). | ADR + code | 1 |
| R-14 | Med | Gateway | The allowlist is coarse. There is no bounded `eth_getLogs` path, and the batch cap of 20 is below ethers v6's default batching (up to 100). | Code | 4 |
| R-15 | Med | Capacity | Self-hosted L1 capacity is thin: node A cannot grow its disk (us-west1 has about 6 GB SSD quota left), node B is only described in a README, node C is not started. | README | 3 |
| R-16 | Low–Med | Product | The native surface is four methods: no transaction, receipt, account, validator or finality reads. The chain ID is a string, not an EIP-155 integer, so wallets and EVM tooling cannot use it. | Live + code | 5 |
| R-17 | Low–Med | Node | The rate limiter refuses new IPs once 100k are tracked, keys on full IPv6 addresses, and uses one global mutex. The header comment (token bucket, dashmap) no longer matches the code. | Code | 2 |
| R-18 | Low | Naming | `/rpc` is CREG-native on `api…` but Sepolia on `explorer…`. A `creg_*` call to the explorer returns a bare `-32601`. | Live | 5 |
| R-19 | Low | Node | `creg_getBlockByNumber` holds the global state read lock across a disk read. Contention with block commit is possible but unmeasured. | Code | 2 |
| R-20 | Med | Liveness | Two of the four validators run on the operator PC. PC off or asleep stops the chain, and the RPC then serves a frozen tip (this makes R-06 worse). Outside the RPC code. | Memory note, 2026-11-15 | 6 |
| R-21 | Low | Tests | `json_rpc.rs` has 4 unit tests. Nothing covers batch, id types, body cap, the rate-limit class or CORS, and there is no post-deploy probe (which is why R-01 went unnoticed). | Code | 0, 2 |
| R-22 | Low | Security | `creg testnet stake-*` passes the signing key to Foundry `cast` on argv (ADR-019 gap 1). | ADR | 6 |
| R-23 | Med | Node | `CREG_TRUSTED_PROXIES` accepts exact IPs only, with no CIDR support, and Docker assigns container IPs dynamically. Trusting Caddy therefore needs a pinned container IP or CIDR support. The faucet already accepts CIDRs (`FAUCET_TRUSTED_PROXY_CIDRS`). | Code (`rate_limit.rs:49`) | 2 |

---

## 2. Phases

Phases 2 and 3 touch different crates (`node` and `common`) and can run in parallel. Phase 1 needs VM access and is yours to run or approve.

### Phase 0: baseline and safety (about 1 day, no production change)

| Task | Closes |
|---|---|
| 0.1 Run the read-only checks in `RPC-ALLOWLIST-REDEPLOY.md` step 0 on the VM. Record image digests, the loaded Caddyfile and the explorer env host names. | R-02 |
| 0.2 Add `chain-registry/testnet/phase-c/probe-rpc-edge.mjs`: read-only, takes host names, asserts the matrices from this document (allowlist, CORS, 405/403/413, `creg_*` basics). Run it now and keep the output as the baseline. | R-01, R-21 |
| 0.3 Test client-IP isolation from two networks, and check `CREG_TRUSTED_PROXIES` on the observer. | R-04 |
| 0.4 Name one canonical deploy path (unified compose plus the r27-stage explorer). Mark the rest as legacy. | R-02 |
| 0.5 Find out who runs the Cloudflare-fronted `/sepolia` upstream and the reth explorer upstream. | R-05 |

**Done when:** the baseline probe output is saved, and the VM state matches or explains the findings above.

**Status, 2026-11-27.** 0.2 is done: the baseline is in [`docs/testnet-heal-evidence/2026-11-27-rpc-phase0-baseline/`](testnet-heal-evidence/2026-11-27-rpc-phase0-baseline/README.md). 0.3 is answered from config (the variable is unset everywhere). 0.4 has an inventory and a proposed canonical path. 0.1 and 0.5 need `chain-registry/testnet/gcp/capture-rpc-edge-state.sh` run on the VM.

### Phase 1: close the open proxy (2–3 days)

1. Execute `RPC-ALLOWLIST-REDEPLOY.md` (rebuild the image if step 0 requires it, stage, promote, then reload Caddy).
2. Patch or delete the legacy nginx configs (R-13).
3. Run the probe against every public host.

**Done when:** the probe passes on `api`, `rpc`, `explorer` and `faucet` hosts: `eth_getLogs` and `web3_clientVersion` return 403, the descriptors say "allowlisted", and `creg_*` is unchanged. **Rollback:** restore the saved Caddy files and the previous explorer digest.

### Phase 2: native RPC fixes (`crates/node`, about 1 week)

| Task | Closes |
|---|---|
| 2.1 A separate `rpc` rate bucket (config and env, default 600/min/IP) so read-only RPC stops sharing the 30/min mutating bucket. | R-03 |
| 2.2 A 429 that is a JSON-RPC error (`-32005`) with `Retry-After`. | R-03 |
| 2.3 Strict request parsing: object or array only, `jsonrpc` must be `"2.0"`, batch up to 20, no response for notifications. | R-08 |
| 2.4 `creg_health` adds tip timestamp, tip age, pending-pool size, sync lag and peer count; add `creg_syncing`. | R-06 |
| 2.5 Per-method request, error and latency metrics; a log line per rejected request. | R-07 |
| 2.6 Rate limiter: evict oldest instead of refusing new IPs at capacity; key IPv6 by /64; correct the header comment. | R-17 |
| 2.7 Measure lock contention on `creg_getBlockByNumber`; shorten the read-lock scope if it shows. | R-19 |
| 2.8 Caddy: pass `OPTIONS` through (or add configured CORS headers) for `/rpc` and `/sepolia`. | R-09 |
| 2.9 Tests: batch, id types, notifications, body cap, rate-limit class, CORS preflight. | R-21 |
| 2.10 CIDR support for `CREG_TRUSTED_PROXIES`, then set it on every observer to the Caddy and explorer network, so the node limits per real client. | R-04, R-23 |

**Done when:** the new tests pass, 31 calls in a minute from one IP return 200, a 429 parses as a JSON-RPC error, and `creg_health` shows tip age.

**Status, 2026-11-27: implemented and tested in Linux Docker.** The laptop has no Visual Studio Build Tools, so the run used a test image that mirrors the builder stage of `chain-registry/Dockerfile` (Ubuntu 24.04, Rust 1.98.1, clang-17).
- Filtered node lib tests (`json_rpc rate_limit metrics public_v4`): **56 passed, 0 failed.** Every new test ran.
- Full node lib suite: **363 passed, 1 failed.** The failure, `authority_adapter::…rejection_sidecar…`, needs a Node.js runtime, which the image does not have. It is unrelated.
- `cargo check --bins`: passes (this includes the uncommitted `main.rs` / `p2p.rs` edits).
- Clippy: no lints in `json_rpc.rs`, `rate_limit.rs` or `metrics.rs`, including their test code. `-D warnings` still fails on lints that predate this change (see "Build health" below).
- rustfmt: both rewritten files formatted, and `metrics.rs` passes `--check`.

What changed:
- `crates/node/src/json_rpc.rs`: strict JSON-RPC 2.0 validation (2.3), batches of 1–20 charged per call, notifications answered with 204, `creg_syncing`, and `creg_health` with `tip_timestamp`, `tip_age_secs`, `pending_pool_size`, `peer_count` and `sync` (2.4). Per-method metrics `creg_rpc_*` (2.5). Chain reads no longer hold the state lock (2.7). 17 unit tests (2.9).
- `crates/node/src/rate_limit.rs`: a separate JSON-RPC bucket, 600/min by default, overridable with `CREG_RATE_LIMIT_RPC` (2.1). The 429 is a JSON-RPC `-32005` error with `Retry-After` (2.2). The table evicts instead of refusing when full; IPv6 is keyed by /64 (2.6). `CREG_TRUSTED_PROXIES` accepts CIDRs, and invalid entries are logged (2.10, code half). 9 new tests.
- `crates/node/src/metrics.rs`: renders the RPC series.
- `testnet/caddy/Caddyfile.unified`: `OPTIONS` on native `/rpc` reaches the node (2.8). This takes effect with the Phase 1 Caddy reload.

- `crates/node/src/consensus_admission.rs:3827` (test code): `crate::ValidatorSetSyncStatus` became `crate::state::ValidatorSetSyncStatus`. The binary crate root has no such re-export, so the `creg-node` test build did not compile at HEAD (`0b0c821`). It compiles now.

Still open in Phase 2:
- **2.10 deploy half:** set `CREG_TRUSTED_PROXIES` on the observers to the Caddy and explorer network, but only after the VM capture shows the node's port is not published to the internet. If it is, an outside client could arrive from the Docker gateway address inside the trusted range and spoof `X-Forwarded-For`.
- **2.8 deploy half:** choose `CREG_CORS_ALLOWED_ORIGINS` for the observers. For a public read-only RPC, `*` without credentials is the usual choice.
- Rate-limited 429s are not counted in `creg_rpc_*` (the middleware answers before the handler runs).

**Build health found during Phase 2 (not RPC, all present at HEAD `0b0c821`):**
- `crates/node/tests/e2e.rs:219` calls `AttestationStore::new()`, which is `#[cfg(test)]` (`consensus_admission.rs:451`), so the `e2e` integration test does not compile. The fix needs a decision: make a constructor public, or give the test another way in.
- `cargo clippy -D warnings` fails on a Linux build:
  - an unused `CommandExt` import (`authority_adapter.rs:353`);
  - two structs with unread fields (`consensus_admission.rs:314`, `:328`);
  - a `match_like_matches_macro` (`:717`);
  - two `field_reassign_with_default` (`:1436`, `:1454`);
  - an unneeded `mut` (`bin/creg_node_supervisor.rs:363`).

  The Windows gate in the build notes only knew about the two `authority_adapter.rs` lints.
- The `authority_adapter` sidecar test needs Node.js on the test machine.

### Phase 3: L1 transport and node capacity (`crates/common`, about 1–2 weeks)

| Task | Closes |
|---|---|
| 3.1 Return a quorum read as soon as a strict majority agrees; cancel the rest. Test with a mock endpoint that hangs. | R-11 |
| 3.2 Per-endpoint health and a cool-down (skip a failing endpoint for N seconds, still count it in the quorum denominator). Add per-endpoint metrics. | R-12 |
| 3.3 Finish node B (Nethermind) acceptance against node A; decide the disk quota for A. | R-15 |
| 3.4 Move observers and validators to at least three CREG-run endpoints (node A, node B, a third), with any vendor only as a last fallback. | R-05 |

**Done when:** a hung endpoint no longer slows quorum reads, a dead primary no longer costs 5 s per call, and no observer env has a single endpoint. Do 3.4 only after 3.3.

**Status, 2026-11-27: 3.1 and 3.2 implemented and tested** (`crates/common/src/l1_rpc.rs`, Linux Docker test image).

What changed:
- **3.1:** a quorum read, in the alloy transport and in `quorum_json_call`, returns as soon as a strict majority of the configured endpoints agree, and drops the endpoints still outstanding. It fails closed as soon as no majority is possible. The denominator, the "unparseable never matches" rule and answer normalisation are unchanged.
- **3.2:** endpoint health lives in a process-wide registry keyed by the ordered endpoint list (`endpoint_health`). The node builds a new provider for almost every operation, so health kept inside one provider would be lost immediately.
  - Failover reads move an endpoint whose transport failed to the back for a cool-down (`CREG_L1_RPC_COOLDOWN_SECS`, default 15 s, 0 disables), still as a last resort. A success clears the cool-down.
  - Quorum reads still go to every endpoint.
  - Primary-only calls still go to the primary exactly once, even while it is cooling down.
- **Counters:** `endpoint_stats(&urls)` returns successes, failures, skips and cool-down state per endpoint, by 1-based position. URLs are never logged or returned.
- **No caller changes:** the node, faucet, indexer, IPFS pinner, `l1-tx` and relayer all pick this up without edits.

Results:
- `cargo test -p common --features l1-rpc l1_rpc`: **30 passed, 0 failed** (20 existing, 10 new). The new tests cover a hung endpoint (mock and real HTTP stub), failing early with no majority, cool-down and recovery, all endpoints cooling, a send to a cooling primary, URL-free counters, and providers built per call sharing health.
- `cargo clippy -p common --features l1-rpc --all-targets -- -D warnings`: clean.
- `cargo check` of `chain-registry-node`, `faucet`, `creg-indexer`, `ipfs-pinner`, `l1-tx` and `chain-registry-relayer`: passes.
- `rustfmt --check`: clean.

Still open in Phase 3:
- 3.3 (node B acceptance, node A disk quota) and 3.4 (at least three CREG-run endpoints in the observer and validator envs). These are operational.
- Render `endpoint_stats` in the node's `/metrics`. This is a small follow-up; it needs the node to keep its configured endpoint list.

### Phase 4: resilience (1–2 weeks, after Phase 1)

| Task | Closes |
|---|---|
| 4.1 A second native-RPC upstream (another observer) behind Caddy health checks. | R-10 |
| 4.2 The explorer gateway gets two upstreams (nginx `upstream` with a backup), or is replaced by the planned Rust gateway (ADR-019 Inc 6) using `common::l1_rpc`. | R-10 |
| 4.3 Edge rate limits that do not depend on the application: Cloud Armor or the load balancer, plus a body cap on native `/rpc`. | R-04 |
| 4.4 Decide the policy for bounded `eth_getLogs` (address filter and block-range cap) and for the batch cap. | R-14 |

### Phase 5: surface and naming (design first, then build)

1. Write a short ADR deciding whether to add read methods (`creg_getTransaction`, `creg_getBalance`, validators, finality) and whether to expose any `eth_*` compatibility.
2. Publish an OpenRPC description of the `creg_*` surface.
3. Fix the `/rpc` naming split: document it, and return a helpful error for `creg_*` on the explorer host. (R-16, R-18)

### Phase 6: operations (ongoing)

1. One deploy script that uploads every Caddy file, records digests, runs the probe, and has no teardown flag. Retire the stale compose variants. (R-02)
2. Scheduled probe plus alerts on tip age, 429 rate, quorum failures and RPC error rate.
3. Mitigation plan for the PC-off halt (R-20), and an in-process signer to replace key-on-argv (R-22).
4. Update ADR-019 remaining gaps with the dates and results.

---

## 3. Decisions needed

1. **Native RPC read limit.** Recommend a separate bucket at 600/min/IP, configurable. Alternative: keep 30/min.
2. **Gateway upstream.** Recommend node A now, node B once it passes acceptance. Alternative: the third-party default.
3. **Bounded `eth_getLogs` on the public gateway.** Recommend no for now (ADR-019 excludes it on purpose); revisit in Phase 4.
4. **`eth_*` compatibility for CREG itself.** Recommend no: CREG is not an EVM chain. Ship an OpenRPC spec and the CLI instead.
5. **Who runs the live `/sepolia` upstream?** Needed before Phase 1 so the new upstream can replace it knowingly. The probe shows a mixed Geth/reth pool behind Cloudflare; the `active_upstreams` line from the VM capture names it.

## 4. Not covered

The consensus and block-production code, the REST API (`/v1/*`), the indexer, and the faucet's own L1 usage were not reviewed. R-20 and R-22 are listed only because they affect RPC availability and the gateway's callers.
