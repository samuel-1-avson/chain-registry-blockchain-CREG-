# Phase C remaining acceptance and public cutover implementation plan

**Execution update, 2026-09-28 13:16 UTC:** At the user's direction, public testnet API, explorer, and root chain-spec routes now select R7. The Caddy config was backed up and hot-reloaded after validation; public API and explorer stats agree on R7 h20 (21 blocks), four active validators, and zero pending transactions. The R27 frontend serves publicly, metrics history returns 61 Prometheus samples across seven series, public JSON-RPC reports R7 and block `0x14`, and the default root spec plus detached signature validate with the R27 Linux CLI. Faucet health remains 200. Eight-node readiness/PBFT/recovery checks passed immediately before cutover. This is a Sepolia testnet route cutover, not a mainnet launch. W5 service-write acceptance, W6 public CLI packaging and positive active-package proof/event acceptance, and the final dossier remain open; the currently installed laptop CLI is older and rejects R7's sorted consensus encoding. See [cutover evidence](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/next-real-package-w3-h19/public-r7-route-cutover-20260928T1316Z.json).

**Execution update, 2026-09-28 04:10 UTC:** The O1 container replacement had dropped its Prometheus and private explorer Docker networks. Both attachments were restored without a restart; the private R7 explorer again serves signed h18 and durable real history. The replacement helper now preserves additional networks and checks O1 history readiness. A new R26 explorer frontend was staged against both real legacy and R7 routes, then promoted publicly with the legacy upstream unchanged. Public browser verification passed 14 routes with no JavaScript errors, and the faucet and Sepolia RPC remain healthy. Public API/explorer still select the separate two-validator legacy h0 chain. W3 pre-application V4 crash proof, remaining W5 service writes, W6 CLI release and coordinated R7 public cutover remain open. See the [repair and deployment evidence](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/next-real-package-w3-h17/README.md).

**Execution update, 2026-09-28 03:19 UTC:** The real R13 `npm:has-flag@5.0.1` publish finalized at shared h17 after a candidate peer-topology repair; its signed certificate and Linux CLI Merkle proof passed. Its original browser publish capture timed out during the split. The owner-signed revoke finalized at shared h18, and the web/CLI named revoke events passed. V4 recovered its matching durable local COMMIT on manual restart, but SIGKILL began just after the capture pause resumed, so the required pre-application crash gate remains open. All eight real nodes are healthy at the same h18 with clear queues/PBFT after V4 disarm. The public explorer/API remain on legacy h0; the faucet's live browser/API/RPC recheck passed. See the [R13 h17/h18 evidence and limitations](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/next-real-package-w3-h17/README.md). No public cutover or chain reset occurred.

**Date:** 2026-09-28
**Status:** Public R7 testnet routes are live as of 2026-09-28 13:11 UTC. W5 service-write acceptance, W6 full CLI distribution and active-package proof/event acceptance, and G7 final dossier remain open. No chain reset was performed.
**Objective:** Close the remaining durability, CLI, explorer, service and release gaps using the existing eight real nodes, then present an evidence-backed public cutover for approval.

This is the current planning document for the remaining work. The earlier [Phase C implementation plan](PHASE-C-IMPLEMENTATION-PLAN.md) contains historical deployment steps. Current recorded status is in [TESTNET-HEAL-STATUS.md](TESTNET-HEAL-STATUS.md) and [EXPLORER-PIPELINE-REPAIR.md](EXPLORER-PIPELINE-REPAIR.md).

## 1. Recorded baseline and limits

The baseline below was refreshed during execution on 2026-09-27. The per-run reports are linked under `docs/testnet-heal-evidence/2026-09-27-phase-c-implementation-start/`.

| Item | Last accepted evidence |
| --- | --- |
| Candidate chain | `creg-phase-c-testnet-2026-09-26-r7` |
| Actual block-zero hash | `0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4` |
| Signed spec `genesis_hash` field | `1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234` |
| Shared finalized tip | h14, `2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa`; includes the real Kleur revoke |
| Node artifact | R22 image `sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3`; scanner profile `f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c` |
| Fleet recovery/readiness | R22 profile rollout and post-publication all-eight readiness passed. All eight share h14; queues and PBFT rounds were clear in the last acceptance capture. Direct V4 local durable COMMIT replay remains open. |
| V3 retained work | Original item preserved as a terminal dead letter; zero active work |
| Dead-letter review | 25 terminal validator records; the original 13 reviewed individually, 12 additions reconciled through logs and metrics |
| V4 crash evidence | Prepared h11 value recovered, then certified canonical reconciliation; local durable COMMIT replay was not observed |
| Candidate explorer | Browser captured and rendered the real h13 Kleur submission, verification and block events through the candidate node proxy; no page errors |
| Local CLI | R7-pinned release CLI built and used for real publication, live named-event capture, package status and Merkle proof verification |
| Candidate package catalog | Five records; all four earlier records and `npm:kleur@4.1.5` are now revoked. Kleur finalized at h13 and its owner-signed revoke at h14; risk score 55 (`elevated`, `review`). |
| Metrics collector | Prometheus is running again on `creg-testnet-vm`; it reuses `monitoring_prometheus-gcp-data`, scrapes the real R7 primary observer every 15 seconds, and returns h14 samples tagged with the R7 chain ID and signed-spec genesis identity. The collector was started without Alertmanager. |
| Public deployment | Final R25 frontend and repaired faucet are live through dedicated pinned Compose projects. Explorer/API/root spec still select separate legacy `creg-testnet-1` at h0; public chain cutover is pending |
| Additional GCP VMs | Four remain `RUNNING`, hosting real R22 nodes and accruing charges; no VM was stopped or deleted |

The actual block-zero hash and the signed spec's `genesis_hash` field are distinct existing identities. Record both, verify their intended use against the signed spec and admission code, and use the authenticated publisher-authorization domain when signing. Do not substitute the block-zero hash for the publisher domain; an earlier request with that substitution was rejected.

Reference evidence: [R21 rollout and acceptance](testnet-heal-evidence/2026-09-26-phase-c-events-r16/r21-rollout/), particularly the fleet restart/readiness, V4 recovery, dead-letter and SSE reports.

## 2. Protocol and operational requirements

The plan follows the [whitepaper](WHITEPAPER.md), especially its deployment note and consensus sections, and [system workflow](CREG_SYSTEM_WORKFLOW.md), especially sections 4.2–4.6, 8.6 and 9.

- Use four voting validators and four non-voting observers. For this roster, package approval, block COMMIT and certified view change require three distinct authorized validator signatures. Observers never contribute to quorum.
- Use actual upstream packages, actual IPFS bytes, real scanner inputs, real signatures, real storage and real Sepolia transactions. Mock responses, simulated nodes, fabricated votes and degraded analysis do not count as acceptance evidence.
- Each node retains its own identity, storage and signing journal. No signing journal may be cloned, edited, erased or restored to an earlier signing state.
- Routine validator rollout and failure drills affect one validator at a time, leaving the other three online. A local timeout does not authorize a replacement proposer without the required certificate.
- Continue the accepted R7 chain from its current tip. Public cutover adopts its accepted history; a new genesis or another reset would require a separate plan and new acceptance evidence.
- Preserve source manifests, image archives, failed attempts and existing dead letters. Do not run Docker cleanup while builds or deployment depend on its images/cache.
- Keep the retired L1 checkpoint bridge disabled. `submitRollupBatch` is not an accepted anchoring or settlement path. Validator work rewards and sponsored staking are separate workflows.
- Protect operator endpoints and signing credentials. Public explorer/CLI clients receive public data, not operator tokens or private keys.
- Reuse the existing hosts and fresh Sepolia deployment. The four additional VMs remain part of the required fleet; provisioning more machines is not a default step.

| Slot | Role | Host | Region/zone | GCP project |
| --- | --- | --- | --- | --- |
| V1 | Validator | Operator laptop | Local | — |
| V2 | Validator | `creg-validator-1` | `us-east1-b` | `gen-lang-client-0022105784` |
| V3 | Validator | `creg-validator-3` | `europe-west1-b` | `fiffi-500411` |
| V4 | Validator | `creg-validator-4` | `asia-southeast1-b` | `fiffi-500411` |
| O1 | Observer/public edge | `creg-testnet-vm` | `us-central1-b` | `gen-lang-client-0022105784` |
| O2 | Observer | `creg-observer-vm` | `us-central1-a` | `gen-lang-client-0022105784` |
| O3 | Observer | `creg-observer-3` | `us-west1-b` | `fiffi-500411` |
| O4 | Observer | `creg-observer-4` | `africa-south1-a` | `fiffi-500411` |

These placements provide separate machines, identities and regional failure domains. Shared GCP administration and provider infrastructure remain common dependencies; the evidence does not establish independent operators or provider diversity.

## 3. Decisions proposed for review

| Decision | Recommendation | Reason / execution dependency |
| --- | --- | --- |
| V4 COMMIT durability gate | **Require a new controlled capture.** | The prior run did not observe recovery of V4's own persisted COMMIT. Durable signing-state recovery needs direct evidence. This does not require V4 to finalize alone before peer sync. |
| Fresh package | Select a small real upstream package/version absent from R7 and review its manifest, content and publisher authority. | All existing candidate records are revoked; SSE has no historical replay. Prepare one normal publish, then its authorized revocation after shared finality. |
| Metrics retention | Implement durable history for the metrics views advertised by the explorer; propose 30 days, sized against actual disk and scrape volume. | Browser samples are not a durable history service. Reuse the existing observability storage where practical. |
| Service funding | Prepare exact, bounded Sepolia funding transactions after reading live balances and roles. | Faucet token reserve, faucet gas, relayer gas and reward treasury have different requirements. Funding amounts/signatures must be reviewed before execution. |
| Release scope | Complete all requested user workflows before full public cutover. | An explicitly reduced release would need a separate scope decision and truthful disabled-feature reporting. |
| Live quorum-loss exercise | Review whether a separate deliberate outage window is required by the final acceptance checklist. | Ordinary drills retain three online validators. Existing three-signature blocks do not prove behavior during a two-validator outage; do not claim that test passed. |
| Public cutover | Approve a concrete cutover package only after all applicable gates pass. | It must include artifact digests, exact routing/config changes, funding/contract state, rollback and acceptance results. |

No decision above authorizes a mock substitute, weaker quorum, new chain reset or automatic activation of the retired bridge. VRF, ZK and namespace activation changes remain subject to their own signed feature gates.

## 4. Execution order and dependencies

| Work package | Depends on | Deliverable |
| --- | --- | --- |
| W0 — Refresh inventory and baseline | Review of this plan | Current fleet/service/config and funding inventory |
| W1a — Inspect retained validator work and dead letters | W0 | Consistent offline reports classifying retained work, starting with V3 |
| W1 — Review COMMIT recovery and capture boundary | W0 | Reviewed recovery path, observation method and frozen artifacts |
| W2 — Prove CLI publication and named events | W1 and W1a; real package/profile readiness | Shared new publication, CLI/browser events and package proof |
| W3 — Capture V4 durable COMMIT crash recovery | W2 shared publication | Real owner revocation, V4 crash/restart timeline and all-eight convergence |
| W4 — Repair remaining API/explorer/history paths | W0; final verification after W3 | Route contract matrix and persistent metrics evidence |
| W5 — Restore and verify service writes | W0; approved funding/roles; canonical-finality gate for rewards | Faucet, wallet/stake, relayer and reward receipt/restart evidence |
| W6 — Package CLI and validate installers | Final CLI fixes from W1/W2/W4/W5 | Reviewed release artifacts and actual installed-binary results |
| W7 — Assemble final acceptance dossier | W1–W6 | Gate matrix, evidence manifest, current fleet snapshot and runbooks |
| W8 — Prepare public cutover | W7 | Reviewable cutover/rollback package; execution follows approval |

W4/W5 investigation and W6 packaging preparation can proceed while W1–W3 are being prepared. Live writes and validator interruptions remain coordinated so each result has an attributable cause. This document is a dependency plan, not a fixed completion-date promise.

### W0 — Refresh inventory and preserve the accepted baseline

1. Read all eight nodes' chain identity, tip/hash, finalized height, roster, peers, scanner profile, sandbox, PBFT rounds, pending/durable/forced-inclusion queues and dead-letter depth.
2. Inspect actual running image IDs and resolve their index/config/platform representations against the preserved artifact manifest. Record binary versions and source manifests.
3. Check current GCP VM status, IPs, firewall source ranges and HTTP sync peers. Validate the laptop's actual egress address; the earlier V1 stall was caused by a stale trusted source address.
4. Verify signed R7 spec and signature, publisher authorization domain, fresh deployment manifest, Sepolia chain ID, contract code/roles and actual active validator roster. Historical contract addresses in documentation are not deployment authority.
5. Inventory the real API, explorer, spec, IPFS, faucet, relayer, database and observability services: host, image, route, persistent volume, health and dependency.
6. Preserve consistent storage checkpoints or stopped copies where required. A RocksDB inspector must use a stopped copy/consistent checkpoint, not a concurrent writable open of the live database. Build the inspector in the reviewed Linux environment with libclang if the Windows dependency is unavailable.

**Pass:** all eight agree on one accepted tip, roles and signed identity; three-validator quorum remains enforced; active work and unresolved PBFT state are explained; service prerequisites have an explicit owner and status.

**Evidence:** `baseline/fleet.json`, `baseline/images.json`, `baseline/services.json`, `baseline/network.json`, `baseline/contracts-and-balances.json`, storage-checkpoint manifest and key-free configuration fingerprints.

**W0 execution (2026-09-27 00:12 UTC):** the fresh read-only fleet capture passed all eight readiness, recovery-clear and PBFT-clear checks. All eight were on R7 h12 with the accepted block hash and scanner profile. Active durable work was zero, but validator dead-letter depths were V1=7, V2=6, V3=6 and V4=6; observer depths were zero. The summary's `recovery_clear` flag does not classify these terminal records, so W1a is a required audit before another package write. Evidence: [W0 fleet readiness](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w0-fleet-readiness.json). The two GCP inventory calls also confirmed the four additional VMs are `RUNNING`; this confirms VM state, not node readiness.

### W1a — Inspect retained validator work and dead letters

Start with V3, then inspect V1, V2 and V4 so repeated records can be compared across each validator's independent store. Current W0 reports show zero active durable work and 25 terminal dead-letter records across the validators; no retry or cleanup is authorized by those counts alone.

1. Identify each validator's actual R21 data path and retained disk. Create a consistent copy using an available RocksDB checkpoint or stop only that validator container long enough to copy its mounted data. Preserve its identity, environment, signer journal and current data. Keep the other three validators online, do not stop the VM, and do not issue writes during capture.
2. Run the pinned `creg_recovery_inspect` only against the copied data directory. It opens the store read-only and emits finalized-work diagnostics. Hash the source copy and report so the evidence can be tied to the capture without modifying the live database.
3. For every item, record node, work ID, transaction kind/hash, retry/dead-letter reason and timestamps. Compare its transaction against R7's canonical blocks and current package/revocation state; compare V3's records with the same work IDs on the other validators.
4. Classify each item as already canonically included/terminal duplicate, valid uncommitted work requiring an explicit retry proposal, or invalid/stale work that must remain quarantined. Do not clear terminal evidence or replay it just to reduce the displayed count. Any future retry must use the supported admission path and preserve the original record and idempotency identity.
5. If a record is missing from canonical history, conflicts with a certificate, or differs across validator stores, stop before attempting repair and retain the captures for protocol review.

**Pass:** all 25 records are individually accounted for; V3's retained work has a documented disposition; no active work was lost or duplicated; all original journals and terminal records remain preserved. A records-only review can pass with no requeue if all records are already terminal duplicates.

**Evidence:** [W1a dead-letter review](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w1a-dead-letter-review.md), per-validator copied RocksDB directories and diagnostics under `testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w1a-v1/` through `w1a-v4/`, and the [canonical reconciliation output](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w1a-v1/canonical-ledger-review-20260927/dead-letter-review.json).

**W1a execution (2026-09-27 02:35 UTC):** consistent chain database copies from V1–V4 were inspected read-only. All validators had zero active work and 25 terminal records total (V1=7, V2=6, V3=6, V4=6). Live R7 blocks 0–12 and package state confirmed all 25 records match already-finalized transactions; there are 18 distinct work IDs and no unresolved record. V1 has two retained work IDs for the same `is-number` publisher nonce. The four package states are revoked, and the revoke records correspond to the committed escape-html h10 and ms h12 revocations. No item was requeued or cleared. All eight nodes passed a fresh readiness, recovery-clear and PBFT-clear capture after V1's database copy. See [W1a dead-letter review](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w1a-dead-letter-review.md), raw [canonical reconciliation](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w1a-v1/canonical-ledger-review-20260927/dead-letter-review.json) and [post-capture fleet status](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w1a-after-v1-snapshot.json). V3's original capture lacks a contemporaneous metadata JSON file; this is called out in the review, while its database, diagnostics and file hashes are retained.

### W1 — Review COMMIT recovery and prepare a measurable crash boundary

Primary sources: `crates/node/src/pbft_safety.rs`, `main.rs`, `block_producer.rs`, `p2p.rs`, `chain_store.rs`, and `crates/consensus/src/pbft.rs` under `chain-registry/`.

1. Trace the real path: persist the prepared block/certificate, persist V4's own COMMIT decision with a synchronous write, apply/broadcast the local vote, recover it on startup, then reconcile canonical history.
2. Check that recovered prepared values and local COMMIT decisions are authenticated and attached to the correct height/parent/chain before voting or applying incoming blocks. Verify startup ordering relative to P2P and HTTP sync.
3. Reuse the existing `Recovered durable local PBFT COMMIT` marker in `main.rs`. Determine whether a non-secret persistence marker is available immediately after V4's own COMMIT write. Round-wide `commit_count` is insufficient proof that V4 persisted its own vote.
4. Add observation fields only if needed: node, height, parent, block hash, phase, view, original vote identity and durable-write completion. Never include a private key.
5. If the write/crash window cannot be captured reliably, design a bounded, explicitly enabled pause after the real synchronous COMMIT write and before subsequent processing. It must use a genuine prepared certificate and actual storage. Review its guards and ensure it is disabled for release.
6. Repair any discovered safety defect before the drill. Freeze a new source manifest and immutable image tag if node code changes; preserve R21. Roll the reviewed artifact through one validator at a time, retaining its latest data/journal, then align all observers.

**Pass:** reviewers can identify the exact durable local COMMIT boundary and startup recovery order. No signing path bypasses the journal; a prepared block and original vote remain recoverable without changing their value.

**Evidence:** `recovery/source-review.md`, source/build manifest, runtime artifact checks and the reviewed drill script with preconditions, deadlines and guaranteed container restoration.

**W1 execution:** source review confirmed `record_decision` uses a synchronous RocksDB write before in-memory COMMIT application or peer broadcast. Startup restores the prepared value and local COMMIT before P2P and sync workers start. The R22 candidate adds machine-readable durable/recovery markers and a compile-gated, exact-node/chain/height/package crash pause capped at 30 seconds. Its runtime arm fails startup unless the node is the specified testnet validator on the specified chain. The Windows compile is blocked by missing `libclang.dll`; the pinned Linux Docker build completed. The 731-file frozen source manifest matches the current source; the final image is Linux/amd64, runs as `creg`, has the accepted R7 spec signer embedded, excludes the local `testnet` directory, and matches the recorded node/CLI binary checksums. Static review is recorded in [R22 image review](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/image-review.json). No R22 image is deployed yet. The rollout must remain one validator at a time with three other validators online, preserve every node's data and signer journal, and pass fresh all-eight readiness and recovery checks before W2. See [W1 source review](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w1-source-review.md).

### W2 — Prove normal CLI publish and named event delivery

Primary sources: `crates/cli/src/publish.rs`, `watch.rs`, `explorer_tui.rs`, `resolver/src/light_client.rs`, the explorer API/event clients, and `explorer/scripts/capture-real-explorer-events.mjs`.

1. Select a real upstream canonical ID absent from R7. Save package metadata, tarball, upstream integrity verification, manifest and content hash. Upload/pin the actual bytes to the real IPFS service and verify retrieval.
2. Check exact package/version coverage in the currently mounted OSV snapshot and the agreed scanner profile. If coverage is absent, fetch a real source response and prepare/review a new pinned profile. Propagate it consistently before voting; absence is not evidence of zero advisories.
3. Validate publisher stake, key ownership, fresh authorization nonce/expiry and the authenticated R7 domain. Respect the existing namespace authority activation policy.
4. Start the rebuilt CLI watcher and real browser event capture before submitting through the real CLI publish command. Preserve stdout/stderr and the signed request, excluding private credentials.
5. Capture `package_submitted`, genuine validator votes, `block_produced` and `package_verified`. Confirm the actual CLI renders the canonical ID, committed block reference and timestamp. Heartbeats alone do not pass this gate.
6. Wait for the publication to become finalized on all four validators and all four observers. Independently verify header hash, Merkle root, authorized signature identities, three-signature COMMIT quorum and consensus-grade package evidence.
7. Before revocation, verify the install-eligible package proof with the actual CLI against the signed R7 spec. Record the observed height as **Hpub**, not an assumed h13; execution must start from the then-current tip.

**Pass:** one real CLI-originated publication appears once in canonical history, all eight agree on Hpub/hash/package state, and both actual CLI and browser render its named events. Scanner/risk limitations remain recorded even when the package is approved.

**Evidence:** `cli-write/package-provenance-r11.json`, `cli-publish.log`, `cli-watch-publish.log`, `browser-publish.json`, `publication-fleet.json`, `package-proof-api.json`, `block-13-api.json`, `cli-package-proof.log` and `cli-chain-record.json`. The CLI did not persist its in-memory wire request to a separate file; the finalized proof response preserves the canonical publication authorization, signer signatures and manifest without private keys.

### W3 — Prove V4 local durable COMMIT recovery using owner revocation

Prerequisites: W2 passed; all eight share Hpub; the other three validators are healthy; the real V4 journal and container are identified; a bounded restore procedure is ready.

1. Keep CLI/browser captures attached. Submit a real owner-signed revocation for the newly published package through one entry point after shared Hpub.
2. Observe V4's genuine PREPARE quorum and its own successfully persisted COMMIT for that next block while V4's canonical store has not yet applied the block.
3. SIGKILL only V4's container at the reviewed persistence boundary. If the block finalized locally before the boundary was captured, do not claim a crash gate; preserve the missed-window evidence and arrange a later genuine transaction if necessary.
4. Leave V1–V3 running. Preserve V4's journal and prepared block unchanged; start the same V4 container with the same identity, data and reviewed artifact.
5. Capture startup recovery of the exact original local COMMIT decision and prepared hash before its first peer-synced canonical application. Record the real sequence and recovered signature verification. V4 need not independently finalize before learning its peers' certificate.
6. Observe retransmission/certified reconciliation, then require all eight to agree on the finalized revocation block **Hrev**, package status and certificate. Check queues, rounds, identity and journal continuity.
7. Review new dead-letter records by payload/reason against committed history. Preserve terminal duplicate evidence; do not replay it to manufacture events or finality.

**Pass:** the run proves a durable local COMMIT existed before the real crash, startup recovered that same decision before peer application, and all eight safely converged with three authorized COMMIT signatures. There is no conflicting value, lost prepared work, signing-state rollback or unexplained stuck round.

**R22 instrumentation constraint:** the deployed hook pauses only in `block_producer.rs` after the local proposer persists its COMMIT. It does not pause in the follower P2P `sign_and_record_commit` path. Do not assume V4 is the proposer for a chosen height or claim a V4-local capture from a block certified by other validators. Before retrying W3, add and review a testnet-only follower-path hook immediately after the synchronous journal write and before `pbft_engine.commit`; bind it to the exact V4, chain, package, height and prepared hash. Build and roll the reviewed artifact with the existing one-validator-at-a-time procedure. A new real publication is required because the Kleur package is now revoked at h14. For the retry, select a fresh real package, wait for its shared finality, attach both live event consumers, then submit its owner-signed revoke and kill V4 only after the exact durable marker and pause marker agree on the same prepared hash.

**W3 execution (2026-09-27):** the owner-signed Kleur revoke was accepted and committed at h14, hash `2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa`. The block proposer and all three COMMIT signers were validator-3, validator-1, validator-2 and validator-3 respectively. The CLI and browser each captured and rendered `package_revoked` and `block_produced`. The all-eight post-block capture and the post-disarm capture both passed at the same h14 hash with no active work or PBFT rounds. This proves revocation finality and event delivery; it does **not** pass the V4 durable-COMMIT recovery gate. The first arm attempt failed closed because the helper used a `0x`-prefixed capture genesis value while `NodeConfig` compares the normalized 64-character value. Its rollback restored the same R22 container. The corrected arm started, but h14 selected validator-3; the V4 monitor observed no matching local-proposer durable/pause marker and did not kill V4. V4 was restored to its original unarmed R22 container with the same persistent data, identity and signer journal. Evidence: [first arm failure and rollback](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w3-v4-capture/arm-attempt-20260927T072510Z.json), [missed V4 marker outcome](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w3-v4-capture/monitor-outcome-no-v4-marker.json), [h14 CLI/browser revoke](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w3-v4-capture/), [disarm report](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w3-v4-capture/disarm-report.json) and [post-disarm all-eight readiness](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/w3-v4-capture/post-disarm-fleet-0805.json).

### W4 — Close API, explorer and durable history gaps

Primary sources: node `api.rs`, `metrics.rs`, `chain_store.rs`, explorer `src/api/` and relevant pages, CLI explorer clients, ingress configuration, and the existing observability compose files.

Build a route contract matrix from actual consumers and servers. For each row, record the upstream service, access scope, response shape, chain identity, error behavior, repair and real verification evidence.

| Path group | Required result |
| --- | --- |
| Health/stats/finality | Canonical CREG height/source distinct from L1 progress; consistent chain and genesis identities |
| Blocks/transactions/packages/proofs | Genesis and cursor boundaries correct; nested records mapped; publication and revocation distinct; revoked proof eligibility represented accurately |
| Search/validators/addresses | Canonical IDs, bounded search, real consensus roster, proposal history, token decimals and stake units correct |
| Pending/recovery | Pending admission, durable finalized work, forced inclusion and dead letters remain distinct; protected endpoints retain their access controls |
| SSE/CLI console | Public SSE and compatibility alias work; payload envelopes render; reconnect refreshes canonical state even when a transient event was missed |
| Wallet/faucet/relayer UI | Browser and CLI use the actual intended service bases; user actions return real status/receipts |
| History | Durable observations retain node/chain/genesis, timestamp, source and sample gaps; restart preserves prior samples |
| Disabled bridge/features | Explicit disabled/unavailable state; no invented anchors, timestamps, settlement or reward success |

For metrics history, inventory the existing Prometheus/database storage before introducing another backend. Mount persistent storage, define retention and capacity, implement the node/history API integration and connect the explorer to real range queries. Expose a reviewed public subset where needed without distributing operator credentials. Start history at the first genuine stored observation; do not backfill fabricated samples from current counters.

**W4 live findings (2026-09-27):** candidate R7 is at h14 while the public API/explorer still serve the legacy `creg-testnet-1` chain at h0; public cutover is pending. Prometheus is restored on the edge VM using the existing named TSDB volume; its R7 target is `up`, `promtool` accepts the config, an earlier range query returned six real h14 samples at 15-second intervals, and the 10:48 UTC live recheck returned 21 real h14 samples over five minutes with exact R7 `chain_id`, signed `network_genesis_hash` and `metrics_source` labels. The observer container was not restarted. The monitored volume is the same `monitoring_prometheus-gcp-data`; no volume reset or backfill occurred. The profile uses 14-day retention, so the implemented public ranges stop at 7d pending a later retention/capacity decision. Source now implements the fixed-series public history API backed by Prometheus, adds the real total-stake gauge, documents the public `range` contract, and connects the web Metrics page and CLI TUI to server-retained samples. `cargo check --locked -p chain-registry-node -p resolver` and `cargo check --locked -p chain-registry-cli` pass in Ubuntu 24.04 WSL with libclang and protoc installed; `npm run build` passes. The first full `cargo build` in WSL stopped with I/O/read-only errors. The retry completed successfully in the isolated libclang 17 builder in 77m55s, producing `creg-node` and `creg` under `E:\creg-phase-c-build-20260927\target\debug`; Cargo exited 0 with a future-incompatibility notice for `sqlx-postgres v0.7.4`. No binaries have been staged to the fleet. The API and explorer source have not been rolled out to candidate or public services, so end-to-end API history, explorer rendering from remote Prometheus, and collector/API restart retention remain acceptance work. The CLI package has not been released. See the [collector restoration evidence](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/api/metrics/collector-restoration.md) and [W4 route contract matrix](testnet-heal-evidence/2026-09-27-phase-c-implementation-start/api/route-contract-matrix.md). W4 remains open until the candidate rollout proves those paths against real nodes and the other advertised node/API paths are captured.

**Pass:** every advertised data view has a functioning authorized path to real data or a truthful unavailable state. Required history returns actual samples before and after a collector/API restart, and deliberate collection gaps remain visible. Preserve the earlier h0 public context until W8.

**Evidence:** `api/route-contract-matrix.md`, real response captures, actual browser/CLI checks, `metrics/retention-and-storage.md` and restart/range-query results.

### W5 — Restore real faucet, wallet/stake, relayer and reward workflows

Use `testnet/phase-c/deployment-manifest.json` and the authenticated signed R7 spec as inputs, then verify deployed code, custody and permissions on real Sepolia. Source manifest addresses are not a substitute for live role checks. Reuse the fresh authorized keys; retired exposed repository keys are not valid operational credentials.

**Faucet update (2026-09-27):** source now rejects a signer/address mismatch,
requires PostgreSQL, and reserves address/IP cooldowns atomically before sending.
The unified and cloud-edge Compose profiles include persistent PostgreSQL.
The code compiles and both Compose profiles validate. A standalone Linux/amd64
image is pinned at
`us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-faucet@sha256:360c124090e5c3a5c448a69e9838bb2c1755ed18f6d32732bff6498d4cdd5ed5`
and has been pulled to the public-services VM, but not started. The public
faucet still returns 502: its container has no signing key. A read-only balance
check found the available secret-derived address has zero tokens on the legacy
public faucet contract; a different configured faucet address is funded but
has no matching known secret. The faucet remains blocked until a matching
funded key or reviewed transfer is available and the database credential is
provisioned. No token transfer was made. See the
[deployment record](testnet-heal-evidence/2026-09-27-explorer-faucet/deployment-status.md).

**Superseding faucet update, 2026-09-28:** the dedicated public faucet is now live, funded and serving HTTP 200. An earlier genuine 1,000-CREG claim, receipt and restart cooldown passed; the fresh browser/API/Sepolia RPC check also passed. The preceding 502 paragraph is retained as historical failure evidence. Wallet/stake, relayer and reward acceptance remain separate open W5 work.

| Workflow | Implementation and acceptance requirements |
| --- | --- |
| Faucet | Resolve `/api/network`, `/api/challenge`, `/api/drip`, balance/stats/health routing; verify token/gas reserves; use persistent PostgreSQL cooldown state and concurrency control; a real drip has the expected receipt and balance delta; duplicate/restart requests cannot bypass limits |
| Wallet and direct staking | Real wallet connection and Sepolia chain selection; correct fresh token/staking addresses and decimals; actual allowance/stake receipts and finalized contract events; browser/CLI/API show the same balance, role and stake |
| Sponsored-stake relayer | Resolve `/v1/relayer/policy`, quote, sponsor and status routes; fund approved gas budget; validate real signed intent/permit/domain/nonce; persist request/tx/replay state across restart; a real receipt yields one stake change and replay cannot cause a second |
| Validator rewards | Review authorized governance/staking caller path; derive real recipients/committee evidence; emit only after durable canonical package finality; persist work ID/outbox/tx/receipt state; enforce duplicate prevention across nodes and restarts; actual finalized receipt and accounting prove one payout |

Primary sources include `crates/faucet/src/main.rs`, `crates/relayer/src/main.rs`, CLI `faucet_client.rs`, `stake.rs`, `relayer_cli.rs`, the real wallet provider, `node/src/validator_pipeline.rs`, `contracts/ValidatorRewards.sol` and related governance/staking contracts.

Prepare a funding sheet with current balances, required transfers, token/gas units, recipients, transaction drafts, authorized signers and spending bounds. Treat a lack of reserves or an unauthorized caller as a real blocker. Do not fund mainnet or purchase assets as an implicit part of testnet validation.

A reward outbox needs a stable identifier bound to the chain and genuine finalized work. Per-node deduplication alone cannot prevent several validators paying the same work. Review the contract's existing capabilities; implement an authorized single distribution path or reviewed contract-level idempotency as needed. Any contract/role change requires a concrete proposal and its actual governance approvals before execution.

Use a separate fresh publication for the reward trial if needed, retaining verified state until its receipt is confirmed. Distinguish disabled/skipped, queued, submitted, confirmed and failed. An HTTP success, submitted hash or vote-quorum log is not proof of a finalized reward payment.

Perform publisher-role stake checks with a dedicated real account so the accepted four-validator voting roster remains stable. Do not accept a wallet stub, fake receipt, fabricated balance or in-memory-only restart result as evidence.

**Pass:** all requested write workflows have real receipts, expected accounting, truthful UI/CLI status and durable retry/replay behavior. The retired checkpoint bridge remains disabled.

**Evidence:** `services/routes-and-health.json`, `services/funding-review.md`, actual faucet/stake/sponsor/reward request and receipt records, post-restart replay checks, and browser/CLI results with secrets excluded.

### W6 — Package the corrected CLI and verify actual delivery

Primary sources: `.github/workflows/release-binaries.yml`, release assurance scripts, `chain-registry/scripts/install-creg.ps1`, `install-creg.sh`, CLI build/version/config handling and signed-spec verification.

1. Freeze final source, toolchain and dependencies. Build release artifacts with a real release version and checksum manifest; verify the embedded spec-verification trust anchor against the signed spec selected for release.
2. Build each advertised platform through its actual toolchain. Do not claim a platform is verified without a real runner/host result. Retain artifacts and reproducibility results.
3. Stage installer/package delivery for review. Verify download URLs, artifact names, integrity/trust checks, executable permissions, configuration preservation and version reporting.
4. Install the reviewed candidate artifact on the laptop and an existing real Linux host. Check executable resolution, including both Windows paths `~/.cargo/bin/creg.exe` and `~/.creg/bin/creg.exe`, so an older binary cannot mask the installed version.
5. Repeat block/spec/proof reads and a real watcher event with the installed binary. Before cutover, explicit candidate selection must remain distinguishable from the still-legacy public default.
6. Publish only the reviewed release artifact after the release gate is accepted. Coordinate public defaults/version pointers with W8.

**Pass:** installed binaries match the approved artifact hashes/version and perform the accepted workflow. No supported download/default path silently mixes the legacy chain, fresh contract manifest or R7 authorization domain.

**Evidence:** `release/source-manifest.json`, platform/checksum manifest, installer captures, executable-resolution results, installed CLI reads/events and rollback package.

### W7 — Assemble final acceptance and operational handover

1. Produce a gate matrix with evidence for each requirement below; retain failed attempts and explicit limitations.
2. Refresh all eight nodes after the last relevant code/config change. Reuse prior passing evidence only when its artifact, scanner profile, chain identity and affected behavior remain valid.
3. Independently verify new block hashes, roots, distinct authorized signatures and shared package state on validators and observers. Check active queues, PBFT rounds, peer/sync health, isolated nsjail, profiles and persistent identities.
4. Recheck finality after any later publication used for rewards or installed CLI events. H12 is the baseline; the accepted cutover tip will be the actual later tip.
5. Review remaining proposer/view-change/observer-recovery requirements against existing real evidence. Add a missing drill only when needed; keep three validators online during normal drills. Any proposed two-validator outage needs its separately reviewed scope before execution.
6. Update status, explorer repair notes, release notes, whitepaper deployment note and system workflow to match observed deployment and disabled features.
7. Write runbooks for restart, safe image rollback, scanner updates, failed transactions, dead letters, funding replenishment, HTTP sync/firewall drift and incident recovery.

**Pass:** each applicable release gate has attributable evidence; outstanding issues and disabled capabilities are explicit; the accepted fleet is stable and ready to support the public routes.

### W8 — Prepare and review public cutover

1. Preserve latest legacy public chain hashes, logs, configuration, database state and recovery evidence. Record the accepted R7 tip, signed spec and fresh contract roles separately.
2. Prepare the exact observer/API/RPC, explorer, IPFS, spec/signature, faucet/relayer and CLI default changes. Derive the active contract addresses from the approved manifest and signed-spec context.
3. Select the already-synced accepted observers for public reads; verify health and peer-sync reachability before edge changes. Keep operator-only endpoints protected.
4. Define cache/service-worker invalidation and network/contract labels so public clients cannot combine cached h0 data with R7 data.
5. Document rollback to the retained public edge configuration and reviewed images. Preserve the latest canonical data and signing journal; rollback never restores earlier signer state. Consider any storage migration before allowing image rollback.
6. Present the concrete artifact/config diff, funding/role results, evidence dossier, maintenance sequence and rollback criteria for final cutover approval.
7. After approval, apply the staged changes and verify public web/CLI/API/spec agreement, actual service writes/events, peer health and the agreed stabilization window. Retire legacy signers/services only after evidence and custody review.

**Pass:** public endpoints and supported installed clients agree on the accepted R7 chain, actual genesis, fresh contracts and final tip; all eight nodes remain healthy; all released user workflows work from public entry points. The accepted chain is preserved without another genesis reset.

## 5. Release gate matrix

| Gate | Required evidence | Current status |
| --- | --- | --- |
| G0 — Fresh baseline | Current eight-node identity/profile/image/queue/role snapshot | All eight real R7 nodes reported shared h20, R14 scanner profile, synced status, and clear readiness/recovery/PBFT state. Service and funding inventory still needs completion |
| G1 — COMMIT durability | Review of persisted local vote and startup recovery ordering | V4's matching h20 local COMMIT was durably logged and replayed before P2P listener startup and peer sync; ANSI-stripped audit passed. Evidence: `next-real-package-w3-h19/v4-r14-h20-replay-marker-audit-ansi-stripped.json` |
| G2 — CLI publication/events | Fresh genuine CLI publish, named events, all-eight certificate/proof agreement | Prior real h13 publish/proof and h14 revoke passed. R14 CLI/browser named revoke and block-event capture passed. All eight packages are currently revoked, so a fresh package is still required for active-package proof acceptance |
| G3 — V4 crash recovery | Local COMMIT persisted before kill, recovered before peer apply, all-eight convergence | Passed for h20: corrected log audit shows durable local COMMIT replay before P2P startup, followed by peer sync and canonical insertion; subsequent all-eight h20 readiness passed. Preserve the earlier false-negative report with its corrected audit |
| G4 — Explorer/API/history | Real route matrix and persistent history across restart | R27 is public and routes to R7. Fifteen candidate browser routes passed; after cutover the public API/explorer stats agree on R7 h20, public metrics history returns 61 Prometheus samples across 7 series, and the root spec/signature validate. A fresh public active-package proof/event capture remains open |
| G5 — Service writes | Actual faucet/stake/sponsor/reward receipts and durable replay control | Faucet real claim, funding and persistent cooldown passed. Relayer durable journal source repair and node reward-safety change compile; no real sponsored-stake receipt/restart acceptance or finalized durable reward dispatch exists yet |
| G6 — CLI distribution | Versioned approved artifacts, installer checks and actual installed behavior | The R27 Linux candidate validated the public root R7 spec signature and read real h20. Positive proof of a fresh active package, final release metadata, Windows rebuild/installers and fresh event acceptance remain open. The currently installed laptop CLI was confirmed older and rejects `creg-sorted-json-v1`; distribute the corrected build before treating client release as complete |
| G7 — Acceptance dossier | Applicable gates, final fleet snapshot and runbooks | Open |
| G8 — Public cutover | Approved configuration/artifact package and successful public checks | Core public testnet API/explorer/spec routes switched to R7 at user direction; public checks passed. Full release acceptance is still open on G5/G6/G7 |

The core R7 routes are live. Remaining G5/G6/G7 gaps prevent calling the full CLI/service release accepted; keep those limitations visible while completing them.

## 6. Evidence, stop conditions and cost control

Create one execution directory under `docs/testnet-heal-evidence/` with the actual run date and subdirectories named above. Every report should identify UTC capture time, node/role, chain/spec/artifact/profile, starting and final height/hash, real request or transaction ID, observer/verifier and pass/fail/blocked status. Keep secret values and private signing material outside reports.

Use bounded monitoring derived from actual configured phase/view timeouts. Proposed initial limits for review are 15 minutes per publish/revoke probe and five minutes per container recovery; adjust before execution when current configuration requires it. An exceeded deadline is preserved failure evidence, not permission to reduce quorum or change chain identity.

Stop dependent writes/drills and preserve the original state when any of these occurs:

- Conflicting finalized/prepared hash, invalid certificate, mismatched signer/chain identity or attempted signing-state rollback.
- Fewer than the required three validators available during a routine rollout/drill.
- Lost prepared work, unexplained active queue growth, profile degradation or an unresolved round beyond the agreed deadline.
- A contract call uses an unauthorized/exposed key, the wrong network/domain, or a duplicate payout/stake occurs.
- Public/operator route scope changes unexpectedly, a release checksum/trust anchor fails, or rollback would require restoring stale signing state.

Restore the interrupted V4 container using its current preserved data and reviewed artifact. Diagnose the failure before resuming downstream work; retain unsuccessful captures beside later successes.

Track the four billable VMs during execution and build waits, along with spend in both GCP projects, retained disks and any static IP charges. Existing budget alerts are not spending caps. Use the already-provisioned fleet; any resizing, new resource creation or extended-idle shutdown should have a concrete cost/state-preservation decision. No automatic stop/delete of required validators or observers is included in this plan.

## 7. Review checklist

- [ ] Accept direct V4 local COMMIT recovery as a mandatory gate.
- [ ] Review the fresh real package and its publisher/scanner prerequisites before the live write.
- [ ] Accept the normal publish → shared finality/proof → owner revoke/V4 crash sequence.
- [ ] Confirm full requested workflow scope and any deliberately disabled capabilities.
- [ ] Review history retention/backend and exact service funding/authorization proposals.
- [ ] Resolve whether a separate real quorum-loss outage is required without weakening routine three-validator availability.
- [ ] Review release platform scope, trust anchors and installer/default behavior.
- [ ] Require the completed acceptance dossier and concrete rollback package before public cutover approval.

**Current execution:** W1a and the one-at-a-time R22 rollout are complete. The R11 real scanner profile is active on all eight nodes. W2 passed: `npm:kleur@4.1.5` was submitted through the rebuilt CLI, named CLI/browser events were captured, h13 finalized with three distinct COMMIT signatures, the signed-spec light-client proof passed, and its owner-signed revoke finalized at h14 across the fleet. Its risk score is 55 (`elevated`, `review`): the official OSV query found zero advisories, while static/behavioral analysis reported color-environment reads and a medium deep-scan warning; preserve that limitation in later acceptance decisions. W3 still needs a V4 follower-path durable-COMMIT recovery capture. W4 source changes compile and the restored collector is producing genuine R7 samples, but candidate rollout and restart/range-query verification remain. No chain reset, public cutover or VM shutdown occurred; the four billable GCP VMs remain running as required fleet hosts.

**Subsequent production pipeline execution, 2026-09-27 21:34 UTC:** W4 history
range queries, actual browser rendering and collector/O1 restart retention now
passed. Final R25 proxy/cache/RPC repairs are deployed publicly against the
unchanged legacy chain and verified privately against R7. The dedicated real
faucet received the exact approved 10,000-CREG transfer; a genuine 1,000-CREG
claim and persistent cooldown after database/faucet restart passed. Source
review identified additional relayer/reward and CLI release defects, preserved
in [service repair review](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/SERVICE-REPAIR-REVIEW.md).
The corrected Windows candidate installs and validates the signed R7 spec;
Linux portable CLI and follower-capture node builds completed successfully. The
portable CLI ran on the actual Ubuntu 22.04 O1 host and read R7 h14/h13; its
canonical package-ID repair is rebuilding. The pinned R25 node image passed an
O2 observer canary after its P2P connections recovered. The first startup
check failed on zero peers and was retained; a separate recovery capture passed.
V4 now runs R25 with its signing journal, h14, identity and six terminal
records preserved; all eight nodes passed the subsequent readiness snapshot.
V4 local COMMIT replay remains unproven. A real
new upstream package and OSV response are prepared, not published. The full
R7/API/spec/contract/default cutover remains behind applicable gates. A limited
read-only R7 release scope choice has been requested and remains unanswered;
silence is not approval to waive a gate. See the [current deployment audit](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/PIPELINE-AUDIT.md).

**Latest blocker work, 2026-09-28:** The R14 eight-node h20 readiness capture and
V4 local-COMMIT replay audit now pass; the previous G0/G1/G3 descriptions above
supersede the older h18 and failed-capture notes. Explorer image R27 is built,
pushed and staged on O1, and its 15-route browser capture passed against real R7
data. The public edge is intentionally still on the legacy chain: its API reports
h0, while the R27 candidate reports R7 h20 and 61 durable Prometheus samples.
The production faucet health, challenge and network endpoints currently return
HTTP 200 and its funded real mode is healthy.

W5 source work now replaces the relayer's in-memory nonce/status/quota state with
a Sled journal that atomically reserves nonce, quotas and request status before
broadcast, and suppresses the node's premature, unauthorized reward send while
reporting skipped emissions. The relayer passed Windows `cargo check`; the node
passed Linux Rust 1.90 `cargo check --locked -p chain-registry-node` after the
Windows toolchain's missing `libclang.dll` was worked around in an isolated Linux
builder. These changes are not deployed. The relayer still needs a signed-
transaction recovery/outbox design for ambiguous broadcasts, persistent
production data-dir verification, and a real Sepolia stake receipt plus
restart/replay capture. Rewards still need a canonical-finality outbox, an
authorized single dispatcher and duplicate protection before dispatch can be
enabled. W6's corrected parser passes Windows `cargo check`; the Ubuntu 22.04
Linux candidate completed and read real R7 h20 on O1, verified the signed R7
spec, and returned the exact canonical revoked-package status. Final release
packaging, fresh active-package proof, Windows artifact/installers and fresh
client-event acceptance remain open. No public cutover or chain reset was
performed; the four billable
GCP VMs remain running.

**Live blocker recheck, 2026-09-28 11:05 UTC:** The public explorer API returned
HTTP 200 but still identifies `creg-testnet-1` at finalized h0 with two active
validators. The staged O1 route returned HTTP 200 for
`creg-phase-c-testnet-2026-09-26-r7` at h20 with four active validators and no
pending transactions. The public metrics-history endpoint still returns 404;
the dedicated faucet health endpoint returns HTTP 200 in real mode. Raw route
responses are preserved in
[`public-route-recheck-20260928T1105Z.json`](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/next-real-package-w3-h19/r27-browser/public-route-recheck-20260928T1105Z.json).
The corrected Ubuntu 22.04 CLI candidate has since completed and passed real
O1 h20 and canonical revoked-package reads using the signed R7 spec. Its SHA-256
is `067bd43243eb45f33a40ae802b573f929e80e619bb73ddde45f4b4b82482b4ca`; the
candidate and O1 output are recorded in
[`cli-linux-r27-o1-acceptance-20260928T1136Z.json`](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/next-real-package-w3-h19/r27-browser/cli-linux-r27-o1-acceptance-20260928T1136Z.json).
W6 remains open for positive proof against a fresh active package, approved
release metadata, final Windows artifact/installers and fresh client-event
acceptance. No public routing, installed CLI, node service, or chain state was
changed by these checks.
