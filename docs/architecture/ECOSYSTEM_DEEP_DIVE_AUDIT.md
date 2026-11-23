# CREG Ecosystem Beyond the Validator — Deep-Dive Architectural & Security Audit

- **Audit date:** 2026-11-23
- **Baseline commit:** `1445d3b` (`main`, ADR-018 Increments 1–7 complete)
- **Scope:** Track 1 — `crates/resolver`, `crates/cli`, `crates/db-sync` (+ `creg-indexer`); Track 2 — `crates/cross-chain`, `crates/relayer`, `crates/faucet`, `deny.toml`; Track 3 — `chain-registry/explorer`, `chain-registry/hub-api`, `chain-registry/hub-web`, and the node's public REST/SSE/WebSocket/OpenAPI surface (`crates/node/src/api.rs`, `events.rs`, `openapi.rs`).
- **Method:** Static code review of the baseline commit against the ADR-018 invariants (`INV-01`–`INV-10`), ADR-005 (evidence/policy), ADR-010 (client freshness) and ADR-017 (hybrid PQ). Findings were confirmed by reading the cited lines. No builds or dynamic tests ran in this phase. Findings that depend on runtime timing are marked **(needs load repro)**.
- **Companion decision record:** [ADR-019](../adr/ADR-019-AGENTIC-ECOSYSTEM-REARCHITECTURE.md)
- **Finding IDs:** `ECA-01` – `ECA-42`
- **Remediation status:** The ADR-019 Increment 1 stop-bleed was implemented on 2026-11-23 and addresses `ECA-07`, `11`, `13`, `14`, `17`, `19`, `23`, `29`, `35`, `36`, `38` and `39`, plus the Increment 1 parts of `15`, `18`, `26` and `34`. See [ADR-019 §13](../adr/ADR-019-AGENTIC-ECOSYSTEM-REARCHITECTURE.md#13-implementation-status) for the per-finding changes, deviations and residual gaps. Correction to `ECA-07`: the per-package REST route (`get_package`) reported quarantined records as `"pending"`, not `"quarantined"`. Only the list and search routes already said `"quarantined"`.

---

## Table of Contents

1. [Executive Summary & Scorecard](#1-executive-summary--scorecard)
2. [Baseline Reconciliation: What ADR-018 Actually Exposes Downstream](#2-baseline-reconciliation-what-adr-018-actually-exposes-downstream)
3. [Component Inventory & Trust Map](#3-component-inventory--trust-map)
4. [Group X — Cross-Cutting & Baseline Integration (`ECA-01` – `ECA-06`)](#4-group-x--cross-cutting--baseline-integration)
5. [Group A — Resolver / Client Verification Spine (`ECA-07` – `ECA-15`)](#5-group-a--resolver--client-verification-spine)
6. [Group B — Developer & Operator CLI (`ECA-16` – `ECA-21`)](#6-group-b--developer--operator-cli)
7. [Group C — PostgreSQL Indexer (`ECA-22` – `ECA-25`)](#7-group-c--postgresql-indexer)
8. [Group D — Relayer, Cross-Chain, Faucet & Supply Chain (`ECA-26` – `ECA-33`)](#8-group-d--relayer-cross-chain-faucet--supply-chain)
9. [Group E — Explorer, Hub & Public Node API (`ECA-34` – `ECA-42`)](#9-group-e--explorer-hub--public-node-api)
10. [`deny.toml` Burn-Down Ledger](#10-denytoml-burn-down-ledger)
11. [Findings Register](#11-findings-register)

---

## 1. Executive Summary & Scorecard

### Overall Verdict

ADR-018 hardened the validator's internals, but **none of its new outputs reach any downstream component today**, and the client spine still trusts the node in ways ADR-018's threat model no longer allows.

1. **The v4 data plane is library-only.** `aggregate_vote_v4_quorum`, `WitnessReplayGuard` and `resolve_quarantine_timeout` have no non-test callers. The node still signs Ed25519 legacy votes, aggregates with the legacy `Verified | Rejected` evidence quorum, never produces `PackageStatus::Quarantined`, and always emits `RevokeV2 { witness: None }` (`ECA-01`). Explorer, hub, CLI and indexer contain zero references to quarantine, witnesses, model families or bounties. That is correct behaviour for data that does not exist yet, but it means downstream work cannot start at the UI end.
2. **One validator can permanently revoke any package for every client.** `RevokeV2` is authorised by a single signature from the owner, a namespace controller, *or any active validator*. The resolver's anti-rollback journal then makes that revocation sticky forever (`ECA-02`).
3. **A `Quarantined` package installs through the documented break-glass.** The resolver maps `"quarantined"` to `Unverified`, and `--unverified` / `CREG_ALLOW_UNVERIFIED=true` installs it from the upstream registry (`ECA-07`). This contradicts ADR-018 open decision #3 ("never auto-promote").
4. **The shipped `creg` binary never runs the light client on install.** Proof verification is compiled only under `common/phase-c-canonical-transactions`, which only `chain-registry-node --features phase-c-real-fleet` enables. A standalone `creg` build skips proof checks during install. Where proofs do run, PBFT commits are checked as Ed25519-only, and the proof never covers current status (`ECA-08`, `ECA-09`, `ECA-10`).
5. **The "ethers → alloy migration" is a deletion.** `crates/cross-chain` has zero `ethers::` call sites. It accounts for 11 of the 50 `deny.toml` ignores. Its "live" L1 relay path fabricates transaction hashes and `Confirmed` receipts against a contract function that does not exist (`ECA-26`, `ECA-27`).
6. **The web edge is unprotected for agentic-era content.** There is no CSP or HSTS anywhere in the repository. The explorer's nginx `add_header` layout silently strips every security header from the SPA document. The explorer renders `RevokeV2` transactions as green "finalized" and quarantined proofs as success (`ECA-34`, `ECA-35`).

### Ecosystem Scorecard

| Dimension | Score (0–10) | Rationale |
| --- | :---: | --- |
| v4 / ADR-018 integration | **1** | Wire types exist in `common`/`consensus`; no producer, no read API, no consumer. |
| Client verification soundness | **3** | Good fail-closed defaults in `install_gate.rs`; undermined by feature-gated proofs, Ed25519-only QC checks, no status proof, and quarantine break-glass. |
| Revocation authenticity | **2** | Single-signer `RevokeV2`, permanent client stickiness. |
| Operator tooling for 4-zone/AI validators | **1** | `creg doctor` has no zone, sandbox-tier or AI-config awareness. |
| Indexer readiness for explorer/hub | **3** | Idempotent upserts and transactions exist; schema is v1-only, `stake_wei` overflows, and there are no migrations. |
| L1 services robustness | **5** | Relayer journal and faucet PG ledger are thoughtfully designed; EOA nonce races, single RPC, fake cross-chain relay. |
| Web edge security | **3** | React escaping helps; no CSP/HSTS, header inheritance bug, spoofable limiters, open RPC proxy. |
| Supply chain | **5** | Blocking `cargo-deny` gate exists; 15 of 50 ignores are removable by deleting dead or retired code. |

### Top 10 Findings at a Glance

| ID | Severity | One-line summary |
| --- | :---: | --- |
| `ECA-01` | CRITICAL | Node never produces v4 votes, quarantine, witnesses or QC'd revocations, so every downstream v4 feature is blocked. |
| `ECA-02` | CRITICAL | Any single active validator can sign a `RevokeV2` for any package; clients make it permanent. |
| `ECA-07` | CRITICAL | `Quarantined` → `Unverified` → installable with `--unverified`. |
| `ECA-08` | HIGH | Standalone `creg` builds skip light-client verification on install; non-phase-C proofs trust a node-supplied validator set. |
| `ECA-09` | HIGH | Light-client PBFT check is Ed25519-only and ignores `vote_v4_active_at`. |
| `ECA-10` | HIGH | No proof of current status: a valid publication proof plus a lying REST status passes. |
| `ECA-11` | HIGH | gRPC fast path stamps the latest version's hash onto any requested version over plaintext `http://host:50051`. |
| `ECA-26` | HIGH | `cross-chain` "live" relay fabricates `Confirmed` receipts for a non-existent `anchorStateRoot`. |
| `ECA-34` | HIGH | Explorer shows `RevokeV2` as green "finalized" and quarantined proofs as success. |
| `ECA-35` | HIGH | No CSP/HSTS anywhere; nginx header inheritance strips all headers from the explorer SPA document. |

---

## 2. Baseline Reconciliation: What ADR-018 Actually Exposes Downstream

The Phase 0 brief names several types that do not exist on `main` under those names or shapes. ADR-019 uses the **actual** names below and proposes additions explicitly where the brief assumed a field.

| Brief name / shape | Actual on `1445d3b` | Location | Consequence for ADR-019 |
| --- | --- | --- | --- |
| `VOTE_V4_DOMAIN` | `VOTE_V4_MESSAGE_DOMAIN = "creg-vote-v4"` | [common/src/vote_v4.rs:17](../../chain-registry/crates/common/src/vote_v4.rs#L17) | Rename in docs only. |
| `TriggerSpecV1` (wire) | `TriggerSpecWireV1 { schema_version, env_overrides: BTreeMap, clock_offset_secs, entry_function, args }`. `TriggerSpecV1` exists only inside the sandbox. | [common/src/vote_v4.rs:46](../../chain-registry/crates/common/src/vote_v4.rs#L46), [validator/src/kernel_sandbox/schema.rs:49](../../chain-registry/crates/validator/src/kernel_sandbox/schema.rs#L49) | Explorer/CLI render `TriggerSpecWireV1`. |
| `ObservedKernelViolationV1` | None on the wire. `TriggerWitnessV1.observed_violations: Vec<String>` is unvalidated. The sandbox has a closed 9-code enum `KernelViolationCode` (`KB-CANARY-EXFIL` … `KB-PROC-UNDECLARED`). | [common/src/vote_v4.rs:147](../../chain-registry/crates/common/src/vote_v4.rs#L147), [kernel_sandbox/schema.rs:131-151](../../chain-registry/crates/validator/src/kernel_sandbox/schema.rs#L131-L151) | `ECA-06`: promote the existing enum to `common`. |
| `SemanticDisposition` | `SemanticDispositionV4` (wire). `SemanticDisposition` is agent-internal. | [common/src/vote_v4.rs:377](../../chain-registry/crates/common/src/vote_v4.rs#L377), [validator/src/ai_agent/runner.rs:34](../../chain-registry/crates/validator/src/ai_agent/runner.rs#L34) | Use `SemanticDispositionV4`. |
| `SemanticVoteV4 { model_family, disposition, confidence_bp, finding_categories }` | `SemanticVoteV4 { disposition, model_family, finding_digests }`. **No** `confidence_bp`, **no** `finding_categories`. | [common/src/vote_v4.rs:387-392](../../chain-registry/crates/common/src/vote_v4.rs#L387-L392) | Pre-activation wire extension proposed (ADR-019 open decision #4). |
| `CregFindingCategory`, `CregFindingSeverity`, `EvidenceClass` | None. Severity is `common::FindingSeverity`; evidence class is implicit. | — | ADR-019 adds `CregFindingCategoryV1` and a pure `evidence_class()` view function. |
| `WitnessAdmissionTracker` | `WitnessReplayGuard` | [consensus/src/vote_v4.rs:332](../../chain-registry/crates/consensus/src/vote_v4.rs#L332) | Rename in docs only. |
| `Transaction::RevokeV2 { quorum_signatures, trigger_witness_digest }` | `RevokeV2 { request: SignedRevocationRequest, evidence_hash, witness: Option<TriggerWitnessV1> }`. `trigger_witness_digest` lives on `ValidatorSignature`. | [common/src/block.rs:262-267](../../chain-registry/crates/common/src/block.rs#L262-L267), [common/src/package.rs:1383](../../chain-registry/crates/common/src/package.rs#L1383) | `ECA-02`: quorum certificate must be added. |
| `SecretRef` `env:` / `file:` / `vault:` | Only `env:` and `vault:` accepted. | [validator/src/ai_agent/config.rs:470](../../chain-registry/crates/validator/src/ai_agent/config.rs#L470) | `file:` is new work (owner-only permission check). |
| `chain-registry/hub` | Does not exist. The hub is `hub-api` (Hono + better-sqlite3 + SIWE) and `hub-web` (Vite/React). | — | Track 3 covers those two. |
| v4 activation | No chain spec in the repo sets `hybrid_signature_activation_height`. v4 is **not active anywhere**. | [common/src/chain_spec.rs:92-97](../../chain-registry/crates/common/src/chain_spec.rs#L92-L97) | Wire additions can still be bundled into the same activation (ADR-018 decision #1). |
| Frozen tests | `consensus_golden_vectors.rs` and `signed_message_formats.rs` contain no v4 types. | `crates/*/tests` | v4 wire extensions do not touch frozen vectors. Only `vote_v4_hybrid_convergence.rs` (not frozen) needs updates. |

**Where v4 data would live on chain once produced.** Per-validator v4 vote fields (`pq_signature`, `voter_key_id`, `semantic_vote`, `trigger_witness_digest`) ride on `ChainRecord.validator_signatures[]` ([package.rs:1375-1383](../../chain-registry/crates/common/src/package.rs#L1375-L1383)). Witness bodies ride on `RevokeV2.witness`. `VoteV4Envelope` itself is a gossip type and is **not** persisted on chain. `DiscoveryBountyAwarded` is an **L1 event** ([ValidatorRewards.sol:118-124](../../chain-registry/contracts/ValidatorRewards.sol#L118-L124)), so nothing on the CREG chain records it.

---

## 3. Component Inventory & Trust Map

| Component | Path | Runtime role | Trust position | Key dependencies of note |
| --- | --- | --- | --- | --- |
| `resolver` | `crates/resolver` | Verdict resolution, install gate, light client, freshness journal, downloader | **Final client-side enforcement gate** on developer machines and CI | `sled 0.34.7`, `tonic`, `reqwest 0.12` |
| `chain-registry-cli` (`creg` + `npm`/`pip`/`gem`/`mvn`/`cargo-shim`) | `crates/cli` | Developer CLI, PATH shims, TUI console, operator helpers | Developer and operator workstations | Links `validator`, `zk-validator`, `ml-validator`, `wasm-sandbox`, `consensus`, `cross-chain` ([cli/Cargo.toml:34-39](../../chain-registry/crates/cli/Cargo.toml#L34-L39)) |
| `db-sync` + `creg-indexer` | `crates/db-sync`, `crates/node/src/bin/creg_indexer.rs` | RocksDB → PostgreSQL mirror | Data source for explorer/hub history | `sqlx 0.8`, `sled` (unused), `tokio-cron-scheduler` (unused) |
| `cross-chain` | `crates/cross-chain` | Multi-chain config, Merkle helpers, "L1 relayer service" | Retired feature (`cross_chain: false`), still linked by CLI | `ethers 2.0.14` (unused) |
| `creg-relayer` | `crates/relayer` | EIP-2612 permit + sponsored staking relay | **Hot L1 wallet** | `alloy 0.6`, `sled`, `k256` |
| `faucet` | `crates/faucet` | Testnet token/native faucet with PG cooldown ledger | **Hot L1 wallet** | `alloy 0.6`, `tokio-postgres` (`NoTls`) |
| `explorer` | `chain-registry/explorer` | React SPA + nginx reverse proxy to node/faucet/RPC | Public web edge | Vite, React Router |
| `hub-api` | `chain-registry/hub-api` | SIWE auth, contribution arena, public status aggregator | Public web edge with sessions | Hono, better-sqlite3, siwe |
| `hub-web` | `chain-registry/hub-web` | Onboarding portal | Public web edge | Vite, React |
| Node public API | `crates/node/src/api.rs`, `events.rs`, `openapi.rs` | REST, SSE, WS, OpenAPI | Read contract for all of the above | `axum`, `utoipa` |

---

## 4. Group X — Cross-Cutting & Baseline Integration

#### `ECA-01` [CRITICAL] The node never produces creg-vote-v4 artifacts, so downstream v4 integration has no data source

- **Evidence:**
  - Votes are built with `..Default::default()`, which leaves `pq_signature`, `voter_key_id`, `semantic_vote` and `trigger_witness_digest` empty ([validator_pipeline.rs:468-480](../../chain-registry/crates/node/src/validator_pipeline.rs#L468-L480)).
  - Quorum uses legacy `aggregate_evidence_votes` with `EvidenceVoteOutcome::{Verified, Rejected}` only ([validator_pipeline.rs:571](../../chain-registry/crates/node/src/validator_pipeline.rs#L571), [:620-750](../../chain-registry/crates/node/src/validator_pipeline.rs#L620-L750)).
  - Finalized records are always `PackageStatus::Verified` ([:667](../../chain-registry/crates/node/src/validator_pipeline.rs#L667)). Rejections always emit `RevokeV2 { witness: None }` ([:743-747](../../chain-registry/crates/node/src/validator_pipeline.rs#L743-L747)).
  - `aggregate_vote_v4_quorum` ([consensus/src/vote_v4.rs:540](../../chain-registry/crates/consensus/src/vote_v4.rs#L540)), `WitnessReplayGuard` ([:332](../../chain-registry/crates/consensus/src/vote_v4.rs#L332)) and `resolve_quarantine_timeout` ([:766](../../chain-registry/crates/consensus/src/vote_v4.rs#L766)) appear only in that module and `consensus/tests/vote_v4_hybrid_convergence.rs`.
  - `PackageStatus::Quarantined` is matched (for string labels) in `api.rs`, `proof.rs` and `db-sync` but is never constructed by node code.
- **Impact:** Every Track 1/3 feature the brief lists (quarantine countdown, witness inspector, Class C diversity, `RevokeV2` certificates, bounty leaderboard) would render empty or fabricated data. The ecosystem cannot be integrated "downstream-first".
- **Remediation:** ADR-019 **Increment 2** (producer completion, pre-activation): wire `aggregate_vote_v4_quorum` behind `vote_v4_active_at(height)`; define the on-chain quarantine lifecycle; attach verified witnesses; expose a read contract (`ECA-05`). This is consensus-side work. It must land before the Track 1/3 v4 UIs, and it can follow the client stop-bleed increment.

#### `ECA-02` [CRITICAL] `RevokeV2` is single-signer; one active validator can permanently brick any package for all clients

- **Evidence:** `validate_revoke_v2` accepts the owner, a namespace controller, **or any active validator** in the height-authenticated set ([namespace_consensus.rs:321-334](../../chain-registry/crates/node/src/namespace_consensus.rs#L321-L334)). The repo's own test asserts `"active validator RevokeV2 should validate"` ([:889-896](../../chain-registry/crates/node/src/namespace_consensus.rs#L889-L896)). `RevokeV2` carries no quorum certificate ([block.rs:262-267](../../chain-registry/crates/common/src/block.rs#L262-L267)). On the client side, `FreshnessStore::persist_package` never lets a `Revoked` record be replaced ([freshness.rs:115-126](../../chain-registry/crates/resolver/src/freshness.rs#L115-L126)), and `evaluate_package_freshness` blocks any later non-revoked status as "anti-rollback" ([:250-258](../../chain-registry/crates/resolver/src/freshness.rs#L250-L258)).
- **Exploit:** A single compromised or malicious validator signs `SignedRevocationRequest` for `npm:react@19.0.0`. Honest proposers include it because the validation rule allows it, and PBFT commits it. Every client that observes it refuses that package version forever, even after governance "un-revokes".
- **Remediation:** Post-activation, accept `RevokeV2` only from (a) the package owner or namespace controller (self-revocation), or (b) a `RevocationQuorumCertificateV1` of `>= 2m/3 + 1` committee reject signatures over a `creg-revoke-qc-v1` message, optionally bound to a peer-verified `TriggerWitnessV1` digest. Clients verify the certificate themselves (ADR-019 `ECO-INV-04`). The journal stays sticky only for **certified** revocations.

#### `ECA-03` [HIGH] The Lane C LLM "intelligence" runs inside the consensus process and publishes free-form LLM prose on the public API

- **Evidence:** `node/src/intelligence/worker.rs:80-81` fetches the IPFS tarball and calls `validator::intelligence::generate_report` from the node process. Reports are served at `GET /v1/public/packages/:canonical/intelligence` ([api.rs:205-208](../../chain-registry/crates/node/src/api.rs#L205-L208), [:2422-2438](../../chain-registry/crates/node/src/api.rs#L2422-L2438)). The explorer renders `executive_summary` and tool-step summaries next to trust status ([PackageIntelligencePanel.jsx:62-105](../../chain-registry/explorer/src/components/PackageIntelligencePanel.jsx#L62-L105)).
- **Impact:** This violates `INV-01`: untrusted tarballs and LLM loops run in a key-holding process. It also creates a prompt-injection-to-UI channel: a package can steer the model into writing "audited and safe" text shown on the official explorer. React escaping stops XSS, but not trust confusion (ADR-005: an LLM is never a safety proof).
- **Remediation:** Move generation into `creg-agent-runner` (Zone 4). Publish only quantized, schema-typed fields (`SemanticVoteV4`, categories, file/byte spans, excerpt digests) on public routes. Free-form prose, if kept at all, goes behind an explicit "untrusted AI text" disclosure and off the default view (ADR-019 open decision #7).

#### `ECA-04` [HIGH] The zone-IPC trust model is unsuitable for extension to operator tooling

- **Evidence:** One 256-bit symmetric token is shared by all four zones ([zone-ipc/src/lib.rs:90-93](../../chain-registry/crates/zone-ipc/src/lib.rs#L90-L93)). `sender` is a self-asserted frame field covered by the same shared key ([:184-190](../../chain-registry/crates/zone-ipc/src/lib.rs#L184-L190)), so any token holder can claim to be `Consensus`. `zone-ipc` keeps no server-side `request_id` replay window, no `SO_PEERCRED` / peer-credential check, and no socket-mode hardening. A repo-wide search for `set_permissions`/`PermissionsExt` finds no zone-socket use. `ZoneResponse::Health` returns only `{zone, status}` ([:287-291](../../chain-registry/crates/zone-ipc/src/lib.rs#L287-L291)).
- **Impact:** The brief asks `creg doctor` to "probe all 4 zones over authenticated zone-ipc". Doing so by giving the CLI `CREG_ZONE_IPC_TOKEN` would let an operator shell, or anything that reads its environment, forge `RunSandboxReplay`, `ForwardAdminHttp` or `RunAgentInvestigation` frames as any zone. A Zone 3 compromise already yields the same capability.
- **Remediation:** Per-link keys derived with HKDF from the boot secret (`info = sender‖target`), so each zone holds only the keys for links it uses. Add a new `ZoneId::Operator` whose link key authorises only `Health` and a new read-only `Capabilities` request. Track `request_id` with a monotonic per-link window. Unix sockets get `0600` plus a `SO_PEERCRED` UID allowlist. Use HMAC-SHA256 instead of the nested-hash construction.

#### `ECA-05` [HIGH] No v4 read contract: OpenAPI, REST DTOs and SSE carry no v4 vocabulary

- **Evidence:** `openapi.rs` contains no `Quarantined`, `VoteV4`, `TriggerWitness`, `SemanticVote` or `RevokeV2` schema. `EventKind` has six legacy kinds ([events.rs:51-58](../../chain-registry/crates/node/src/events.rs#L51-L58)). Package DTOs collapse `Quarantined{reason, expires_at_unix}` into the string `"quarantined"` with no expiry ([api.rs:2205-2207](../../chain-registry/crates/node/src/api.rs#L2205-L2207)). `PackageStatus` is externally tagged (`"Verified"` versus `{"Quarantined":{...}}`, [package.rs:1332-1347](../../chain-registry/crates/common/src/package.rs#L1332-L1347)), so raw-record consumers must special-case shapes. The explorer's generated `types.ts` inherits the gap.
- **Remediation:** A single versioned view model (`PackageStatusViewV1`, `VoteV4View`, `WitnessView`, `RevocationCertificateView`, `QuarantineView`) in a new `crates/registry-view` crate. It is generated into OpenAPI and TypeScript, and emitted as typed SSE kinds (`package_quarantined`, `quarantine_resolved`, `witness_verified`, `package_revoked_v2`), each carrying a monotonic `id:` (block height + index) for `Last-Event-ID` resume.

#### `ECA-06` [MEDIUM] v4 display vocabulary is open-ended and attacker-influenced

- **Evidence:** `observed_violations: Vec<String>` is checked only for count, not membership ([vote_v4.rs:197-199](../../chain-registry/crates/common/src/vote_v4.rs#L197-L199)). A closed nine-code enum already exists, but only inside the sandbox (`KernelViolationCode`, [kernel_sandbox/schema.rs:131-151](../../chain-registry/crates/validator/src/kernel_sandbox/schema.rs#L131-L151)), so the wire layer and consumers cannot use it. `SemanticVoteV4` has no category taxonomy. `reject_reason: Option<String>` is free text. `TriggerSpecWireV1` env values up to 4 KiB each are attacker-chosen in the sense that the agent synthesised them from package content ([vote_v4.rs:73-84](../../chain-registry/crates/common/src/vote_v4.rs#L73-L84)).
- **Impact:** Every renderer (terminal, TUI, web, PG) must independently decide what is safe to show. Unknown codes and long values are display-injection and UI-spoofing vectors (`ECA-17`).
- **Remediation:** Promote `KernelViolationCode` into `common` and add a closed `CregFindingCategoryV1`. Validate both in `validate_wire_budget` / `SemanticVoteV4::validate` post-activation. Renderers map codes through an allowlist. Unknown values render as `UNKNOWN(<sha256-8>)`. Add a `DisplayText` newtype (bounded, C0/C1/ESC/bidi-control stripped) produced at the view-model boundary.

---

## 5. Group A — Resolver / Client Verification Spine

#### `ECA-07` [CRITICAL] `Quarantined` packages are installable through the break-glass path

- **Evidence:** The REST status mapping makes everything other than `"verified"`/`"revoked"` into `Unverified` ([chain_client.rs:154-166](../../chain-registry/crates/resolver/src/chain_client.rs#L154-L166)). `evaluate_install(Unverified, allow_unverified=true)` returns `AllowUnverified` ([install_gate.rs:118-130](../../chain-registry/crates/resolver/src/install_gate.rs#L118-L130)), and `creg install` then delegates to the real package manager against the upstream registry ([cli/src/install.rs:217-219](../../chain-registry/crates/cli/src/install.rs#L217-L219)). `VerdictStatus` has no quarantine variant ([verdict.rs:38-57](../../chain-registry/crates/common/src/verdict.rs#L38-L57)). The freshness journal has none either ([freshness.rs:40-45](../../chain-registry/crates/resolver/src/freshness.rs#L40-L45)).
- **Exploit:** CI pipelines that set `CREG_ALLOW_UNVERIFIED=true` to tolerate brand-new packages silently install code that the committee explicitly held for review.
- **Remediation:** Add `VerdictStatus::Quarantined { reason, expires_at_unix, evidence }`. Hard-block it like `Revoked`, with no `--unverified` override. Once `expires_at_unix` passes, treat it as `Revoked`, mirroring `resolve_quarantine_timeout`. Add a `Quarantined` journal kind that can only move to `Revoked`, or to `Verified` with a certified release. This needs no wire change: the node already returns `"quarantined"`, so it ships in the stop-bleed increment.

#### `ECA-08` [HIGH] Client proof verification is compile-gated on a node-only feature

- **Evidence:** `resolve_id_for_install` runs `light_client::verify_verdict` only when `common::compiled_consensus_encoding().is_some()` ([install_gate.rs:183-187](../../chain-registry/crates/resolver/src/install_gate.rs#L183-L187)). That requires `common/phase-c-canonical-transactions`, which only `chain-registry-node`'s `phase-c-real-fleet` enables ([node/Cargo.toml:115](../../chain-registry/crates/node/Cargo.toml#L115)). `crates/cli` and `crates/resolver` declare no forwarding feature. Without phase C, the light client takes its authority from the node's own response: `ValidatorSet::new(resp.active_validators.clone())` ([light_client.rs:536-539](../../chain-registry/crates/resolver/src/light_client.rs#L536-L539)).
- **Impact:** A standalone `cargo build -p chain-registry-cli` produces a `creg` that installs on an unauthenticated node assertion. Security behaviour flips under Cargo feature unification depending on what else is in the same build invocation.
- **Remediation:** Remove the compile gate. The resolver always verifies against a pinned, signed chain spec (`load_verified_chain_spec`), with the signer pinned at build time and the network selected at runtime. Non-phase-C "trust the node" verification is deleted.

#### `ECA-09` [HIGH] Light-client PBFT verification is Ed25519-only and ignores the activation height

- **Evidence:** `verify_pbft_consensus` decodes every commit signature as `SignatureScheme::Ed25519` and never reads `sig.pq_signature` ([light_client.rs:254-282](../../chain-registry/crates/resolver/src/light_client.rs#L254-L282)). It hard-codes the message format `"creg-pbft-v1:commit:{hash}"` ([:237](../../chain-registry/crates/resolver/src/light_client.rs#L237)) instead of calling `consensus::pbft::pbft_signature_message`, and never consults `ConsensusParams::vote_v4_active_at`. Consensus itself requires hybrid signatures post-activation and rejects classical-only ones ([consensus/src/vote_v4.rs:71-157](../../chain-registry/crates/consensus/src/vote_v4.rs#L71-L157)).
- **Impact:** After v4 activation, a forger who breaks or steals only Ed25519 keys satisfies every client while honest validators reject the block. ADR-017's harvest-now protection does not extend to clients.
- **Remediation:** The resolver calls `consensus::vote_v4::verify_pbft_phase_signature_at_height` for each signature, using `ConsensusParams` from the signed spec. Tolerate individual bad signatures, and count only valid distinct members toward quorum.

#### `ECA-10` [HIGH] No proof of current status or non-revocation

- **Evidence:** The proof endpoint returns `status: "verified"` even for revoked packages, signalling current state only in a separate `package_status` field ([proof.rs:32-37](../../chain-registry/crates/node/src/proof.rs#L32-L37), [:100-102](../../chain-registry/crates/node/src/proof.rs#L100-L102)). The light client checks `resp.status` and the **publication-time** record status, and ignores `package_status` ([light_client.rs:484-502](../../chain-registry/crates/resolver/src/light_client.rs#L484-L502)). `verify_verdict` returns `Verified` whenever the REST status said "verified" and a publication proof exists ([:625-646](../../chain-registry/crates/resolver/src/light_client.rs#L625-L646)). Witnessed-head policy (ADR-010) is inactive unless explicitly configured ([freshness.rs:303-309](../../chain-registry/crates/resolver/src/freshness.rs#L303-L309)).
- **Impact:** A malicious, stale or eclipsed node answers "verified" for a revoked package, and the client installs it.
- **Remediation:** Make current status a proven fact against a fresh, QC-verified tip header. ADR-019 proposes a post-activation `status_root` (sparse Merkle tree keyed by `sha256(canonical)`) in `BlockHeader` with `skip_serializing_if = "Option::is_none"`, which leaves pre-activation header bytes unchanged. The client then verifies a current-value proof plus tip freshness. Until activation, witnessed heads become the install default (open decision #1).

#### `ECA-11` [HIGH] The gRPC fast path confuses versions and downgrades TLS

- **Evidence:** `fetch_verdict` derives `http://{host}:50051` by stripping the scheme and port from `node_url` ([chain_client.rs:14-20](../../chain-registry/crates/resolver/src/chain_client.rs#L14-L20)). It calls `get_latest_version(ecosystem, name)` and stamps the latest **Verified** version's `content_hash` as `Verified` for whatever version was requested ([:29-60](../../chain-registry/crates/resolver/src/chain_client.rs#L29-L60); server side [grpc/server.rs:94-114](../../chain-registry/crates/node/src/grpc/server.rs#L94-L114), [chain_store.rs:2021-2053](../../chain-registry/crates/node/src/chain_store.rs#L2021-L2053)).
- **Impact:** `creg status|audit|verify` report a revoked `express@4.17.1` as VERIFIED whenever `express@5.x` is verified and port 50051 answers. That verdict is cached for 24h. The path also runs over plaintext even when `CREG_NODE_URL` is HTTPS.
- **Install chain, with an honest node:** In a standalone (non-phase-C) `creg`, install fails closed by accident, because gRPC returns an empty `ipfs_cid`. In a phase-C build with node gRPC enabled (`CREG_GRPC_ENABLED=true`; it is off by default and binds `127.0.0.1`, so this mainly affects validator and dev hosts), the chain is:
  1. gRPC returns `Verified` for the revoked version.
  2. `verify_verdict` fetches that version's proof.
  3. `proof.rs` still answers `status: "verified"` for revoked records (`ECA-10`).
  4. The publication-time record passes the light client, and `creg install` installs the revoked tarball.
  Where both conditions hold, this is CRITICAL.
- **Remediation:** Delete the gRPC verdict path from the resolver. Verdicts come only from proof-carrying REST.

#### `ECA-12` [HIGH] The freshness journal's freeze and mix-and-match checks are effectively inert

- **Evidence:** `resolved_at` is client-generated (`Utc::now()` in `chain_client.rs`), and freeze detection compares two client timestamps ([freshness.rs:260-270](../../chain-registry/crates/resolver/src/freshness.rs#L260-L270)). Content-hash pinning fires only when `verdict.resolved_at == previous.resolved_at` ([:271-285](../../chain-registry/crates/resolver/src/freshness.rs#L271-L285)), which essentially never happens.
- **Impact:** A node that swaps the content hash for an existing `name@version`, or serves an old snapshot, is not detected.
- **Remediation:** Pin first-seen `canonical → content_hash` permanently, since a version is immutable once published. Track monotonic `(tip_height, tree_size)` from proven headers and witnessed heads instead of wall-clock time.

#### `ECA-13` [HIGH] The non-install resolve path trusts a 24h unauthenticated cache and drives CI audits

- **Evidence:** `resolver::resolve_id` returns any unexpired cache entry with no proof ([lib.rs:32-64](../../chain-registry/crates/resolver/src/lib.rs#L32-L64)). Verified entries live 24h ([cache.rs:56-61](../../chain-registry/crates/resolver/src/cache.rs#L56-L61)). Users: `creg audit` ([audit.rs:81](../../chain-registry/crates/cli/src/audit.rs#L81), [:462](../../chain-registry/crates/cli/src/audit.rs#L462)), the `creg verify` pre-check ([verify.rs:58](../../chain-registry/crates/cli/src/verify.rs#L58)), `creg status`, and `creg batch`. An unreachable node becomes `Unknown` and is cached for 5 minutes.
- **Impact:** A package revoked after it was cached reads VERIFIED in `creg audit --fail-on-unverified` for up to 24h. That is the exact CI gate teams rely on.
- **Remediation:** One resolution path, the install-grade proof-carrying one, for every security-relevant command. The cache becomes a proof cache keyed by `(canonical, content_hash)` and is re-validated against a fresh status proof.

#### `ECA-14` [MEDIUM] The P2P downloader is unbounded and writes to a predictable shared temp path

- **Evidence:** `reqwest::get(&url).await?.bytes()` has no size cap and no timeout; `ipfs_cid` is interpolated unvalidated into the URL path; losing tasks are not aborted ([downloader.rs:43-60](../../chain-registry/crates/resolver/src/downloader.rs#L43-L60)). The output path is `std::env::temp_dir()/{name}.tgz`, written with `File::create` and handed to the package manager afterwards ([install.rs:188-199](../../chain-registry/crates/cli/src/install.rs#L188-L199), [downloader.rs:65-66](../../chain-registry/crates/resolver/src/downloader.rs#L65-L66)).
- **Impact:** A hostile node can exhaust client memory (no `INV-09` parity). On shared hosts, another user can pre-create a symlink or swap the file between hash check and install (TOCTOU).
- **Remediation:** Stream through an incremental SHA-256 into a `tempfile::NamedTempFile` in a private `0700` directory with a hard byte cap. Validate the CID with the same strict CIDv0/v1 grammar as `AdmissionGate`. Hand the package manager the verified file descriptor's path only after verification, and abort losing futures.

#### `ECA-15` [MEDIUM] The client cache and journal are not crash- or concurrency-safe; `sled` is unmaintained

- **Evidence:** `sled::open(path).expect(...)` panics on corruption or a held lock, and the path falls back to `/tmp` ([cache.rs:13-23](../../chain-registry/crates/resolver/src/cache.rs#L13-L23)). `save_file` removes the journal and then renames, with no fsync and no cross-process lock ([freshness.rs:173-192](../../chain-registry/crates/resolver/src/freshness.rs#L173-L192)). On Windows, `std::fs::rename` already replaces the target, so the pre-delete only opens a window where the anti-rollback journal does not exist. Parallel shims (`npm` + `pip`) lose updates.
- **Remediation:** Replace `sled` with a bounded, single-file JSON store using `write-temp → fsync → rename → fsync(dir)` (no pre-delete) and an advisory lock file (`fs2`-style `LockFileEx`/`flock`). Corruption fails closed and quarantines the file instead of panicking. Retire `sled` from `crates/resolver/Cargo.toml`.

---

## 6. Group B — Developer & Operator CLI

#### `ECA-16` [HIGH] The developer CLI links the whole validator analysis stack and retired crates

- **Evidence:** `chain-registry-cli` depends on `validator`, `zk-validator`, `ml-validator`, `wasm-sandbox`, `consensus` and `cross-chain` ([cli/Cargo.toml:34-39](../../chain-registry/crates/cli/Cargo.toml#L34-L39)). Call sites:
  - `creg advanced …`: local ZK/ML/WASM/sandbox runs ([advanced.rs:12-92](../../chain-registry/crates/cli/src/advanced.rs#L12-L92)).
  - `creg doctor` fleet checks: YARA rules, analysis bundle set, scanner version ([doctor.rs:153-171](../../chain-registry/crates/cli/src/doctor.rs#L153-L171)).
  - `creg osv-snapshot build` ([osv_snapshot.rs:18-19](../../chain-registry/crates/cli/src/osv_snapshot.rs#L18-L19)).
  - `creg relayer …` (`cross-chain`) and the stress test (`consensus::validator_set`).
- **Impact:** Every developer laptop and CI runner that installs `creg` (plus its `npm`/`pip` PATH shims) carries wasmtime, yara-x, arkworks and ethers, the code paths that hold most `deny.toml` exceptions. That is a large attack and update surface for a binary whose job is to check proofs and gate installs. It also breaks the spirit of `INV-01` from the client side.
- **Remediation:** `creg` links only `common`, `consensus` (verification-only, already lightweight), `resolver`, `registry-view`, a new small `ai-config` crate, and `zone-ipc`. Fleet doctor checks move behind the zone `Capabilities` report (`ECA-04`/`ECA-21`). `osv-snapshot build`, `advanced`, `stress` and `relayer` move into a separate `creg-dev` binary crate, or are deleted with their retired back-ends.

#### `ECA-17` [MEDIUM] Untrusted chain strings reach operator terminals unsanitized

- **Evidence:** `creg watch` prints `canonical`, `reason`, `publisher_pubkey` and other fields straight from SSE JSON ([watch.rs:267-330](../../chain-registry/crates/cli/src/watch.rs#L267-L330)). Validator reject reasons and finding descriptions can embed tar paths and other package-derived text. The TUI already emits OSC 52 itself ([explorer_tui.rs:2372](../../chain-registry/crates/cli/src/explorer_tui.rs#L2372)), so target terminals interpret OSC. No C0/C1/ESC stripping exists anywhere in `crates/cli/src`.
- **Impact:** OSC 52 clipboard writes, OSC 8 hyperlink spoofing, title rewriting, or `\r` overwrite of the "✗ REJECTED" label in operator and CI terminals.
- **Remediation:** Every untrusted string passes through `DisplayText::for_terminal()`, which strips `0x00–0x1F` except `\t`, `0x7F–0x9F`, ESC, and bidi overrides `U+202A–U+202E` / `U+2066–U+2069`, and truncates by grapheme count.

#### `ECA-18` [MEDIUM] `creg watch` has an unbounded buffer, and CI mode ignores v4 events

- **Evidence:** `buffer.extend_from_slice(&chunk)` grows without bound until a frame separator arrives ([watch.rs:136](../../chain-registry/crates/cli/src/watch.rs#L136)). `--ci` fails only on `package_rejected | package_revoked` ([:240-243](../../chain-registry/crates/cli/src/watch.rs#L240-L243)).
- **Remediation:** Cap pending bytes at 256 KiB and drop the connection when exceeded. CI-critical kinds become `package_revoked*`, `package_quarantined`, `witness_verified` and `quarantine_expired`. Resume with `Last-Event-ID`.

#### `ECA-19` [MEDIUM] Private keys are accepted on argv, and token amounts are `f64`

- **Evidence:** `creg login --private-key` ([main.rs:500-503](../../chain-registry/crates/cli/src/main.rs#L500-L503)) and `creg testnet stake-publisher|stake-validator --key` ([:872-893](../../chain-registry/crates/cli/src/main.rs#L872-L893)) take keys on the command line, where they are visible in process lists, shell history and CI logs. The same commands take `amount: f64`.
- **Remediation:** Accept `--key-file` or `--key-ref env:|file:|vault:` only, with a hidden deprecated alias that warns and refuses unless `CREG_ALLOW_ARGV_SECRETS=1`. Parse amounts as decimal strings into `U256` wei.

#### `ECA-20` [MEDIUM] Login derives the publisher key from a wallet signature, and the loopback listener allows any origin

- **Evidence:** `derive_ed25519_key_from_signature(&sig_bytes)` turns an ECDSA wallet signature over a fixed, public digest into the Ed25519 publisher private key ([auth.rs:530](../../chain-registry/crates/cli/src/auth.rs#L530)). RFC 6979 makes that signature deterministic, so any page that gets the user to sign the same typed data recovers the publisher key. The loopback callback answers every response with `Access-Control-Allow-Origin: *` and `Access-Control-Allow-Private-Network: true` ([auth.rs:409-419](../../chain-registry/crates/cli/src/auth.rs#L409-L419), [:547-556](../../chain-registry/crates/cli/src/auth.rs#L547-L556)). The state nonce prevents takeover, but any site can detect an in-progress login.
- **Remediation:** Generate the publisher key locally (or with hybrid `creg keygen`) and bind it with a wallet signature over a challenge that includes the new public key. Never derive secret material from signatures. Restrict CORS on the loopback to the configured web-bridge origin.

#### `ECA-21` [LOW / GAP] `creg doctor` has no awareness of 4-zone validators, sandbox tiers or AI config

- **Evidence:** Doctor checks the node, IPFS, keys, GnuPG, a generic "isolated sandbox" and an LLM env toggle ([doctor.rs:151-358](../../chain-registry/crates/cli/src/doctor.rs#L151-L358)). Reusable building blocks exist: `ValidatorAiConfig::validate()` ([ai_agent/config.rs:441](../../chain-registry/crates/validator/src/ai_agent/config.rs#L441)), `resolve_rootfs_strict` / `compute_directory_tree_sha256` with the `CREG-ROOTFS-TREE-V1` domain ([kernel_sandbox/nsjail.rs:76-143](../../chain-registry/crates/validator/src/kernel_sandbox/nsjail.rs#L76-L143)), and `detect_nsjail_ready` ([:538](../../chain-registry/crates/validator/src/kernel_sandbox/nsjail.rs#L538)). They are either buried in the validator crate or unreachable over IPC.
- **Remediation:** ADR-019 Increment 4 adds `creg validator doctor` and `creg ai check|probe`, backed by an extracted `ai-config` crate and the read-only `ZoneRequest::Capabilities` over the operator link (`ECA-04`).

---

## 7. Group C — PostgreSQL Indexer

#### `ECA-22` [HIGH] The indexer schema is blind to v4 and to revocation provenance

- **Evidence:** `validator_votes.vote` is `CHECK (vote IN ('approve','reject'))` with no PQ, key-id, semantic or witness columns ([schema.rs:58-69](../../chain-registry/crates/db-sync/src/schema.rs#L58-L69)). `RevokeV2` collapses into `status='revoked'`, discarding the signer, `evidence_hash` and witness ([lib.rs:72-80](../../chain-registry/crates/db-sync/src/lib.rs#L72-L80)). `blocks` omits `validator_set_hash`, `view_number` and commit signatures. There is no quarantine expiry column, and `Slash` / `ValidatorJoin` / `ValidatorLeave` are dropped ([lib.rs:98](../../chain-registry/crates/db-sync/src/lib.rs#L98)).
- **Remediation:** Schema v2 (ADR-019 §6.3): `package_status_history`, `vote_signatures_v4`, `semantic_votes`, `trigger_witnesses`, `revocations` (with certificate JSONB and signer set), `quarantines`, `validator_events`, `discovery_bounties` (from L1 logs) and an enriched `blocks`.

#### `ECA-23` [HIGH] `publisher_stats.stake_wei BIGINT` overflows above roughly 9.22 CREG

- **Evidence:** [schema.rs:93](../../chain-registry/crates/db-sync/src/schema.rs#L93), duplicated at [sync_worker.rs:248](../../chain-registry/crates/db-sync/src/sync_worker.rs#L248). With 18 decimals, `i64::MAX` wei is about 9.22 tokens, and the validator minimum stake is 100 CREG.
- **Remediation:** Use `NUMERIC(78,0)` for every wei or `uint256` column.

#### `ECA-24` [MEDIUM] Schema drift, no migrations, the cursor outside the block transaction, and silent conflicts

- **Evidence:** The full DDL exists twice: `schema::INIT_SQL` ([schema.rs:6-98](../../chain-registry/crates/db-sync/src/schema.rs#L6-L98)) and `bootstrap_schema` ([sync_worker.rs:90-258](../../chain-registry/crates/db-sync/src/sync_worker.rs#L90-L258)), with no version table. The block is applied in one transaction ([lib.rs:49-117](../../chain-registry/crates/db-sync/src/lib.rs#L49-L117)), but the cursor update is a separate statement ([sync_worker.rs:298-309](../../chain-registry/crates/db-sync/src/sync_worker.rs#L298-L309)). `INSERT … blocks ON CONFLICT (height) DO NOTHING` hides hash divergence. There is no `prev_hash` continuity check. `RotatePublisherKey` uses an unescaped `LIKE '{prefix}%'` ([lib.rs:88-96](../../chain-registry/crates/db-sync/src/lib.rs#L88-L96)): `_` and `%` in names act as wildcards, and `npm:foo` also matches `npm:foobar`.
- **Remediation:** Embedded `sqlx::migrate!` migrations as the single source of truth. Advance the cursor inside the same transaction. Abort on `(height, hash)` conflict or a broken `prev_hash`. Do exact namespace matching through a `package_namespace` column.

#### `ECA-25` [MEDIUM] Indexer topology: an in-process mode inside consensusd and a dedicated mode that reopens RocksDB per block

- **Evidence:** The node can run `SyncWorker` in-process ([node/src/main.rs:969-974](../../chain-registry/crates/node/src/main.rs#L969-L974)), which puts PostgreSQL credentials and `sqlx` into the consensus process. The dedicated `creg-indexer` calls `ChainStore::open_read_only` on **every** `tip_height()` and every `get_block_by_height()` ([creg_indexer.rs:15-31](../../chain-registry/crates/node/src/bin/creg_indexer.rs#L15-L31)), links the whole `node` crate, and binds `0.0.0.0:8084` ([:56-57](../../chain-registry/crates/node/src/bin/creg_indexer.rs#L56-L57)). `sled` and `tokio-cron-scheduler` are declared but unused in `db-sync`.
- **Remediation:** A standalone `crates/indexer` that opens a RocksDB **secondary** instance once (`try_catch_up_with_primary`), or pulls and verifies over the public API with the light-client rules when remote. Delete the in-process mode. Bind health to loopback. Remove the dead dependencies.

---

## 8. Group D — Relayer, Cross-Chain, Faucet & Supply Chain

#### `ECA-26` [HIGH] The `cross-chain` "live" L1 relay fabricates confirmed receipts for a non-existent contract function

- **Evidence:** With `dry_run = false`, `process_next_batch` never contacts an RPC. It synthesises `tx_hash = sha256("live-tx:{height}:{root}")` and records `RelayTxStatus::Confirmed { block_number: 6_840_120 }` with `gas_used: 48_520` ([l1_relayer_service.rs:153-184](../../chain-registry/crates/cross-chain/src/l1_relayer_service.rs#L153-L184)). The calldata targets `anchorStateRoot(uint64,bytes32,bytes32)` with a hand-written selector `0x6e96996d` ([:92-130](../../chain-registry/crates/cross-chain/src/l1_relayer_service.rs#L92-L130)). No such function exists in `chain-registry/contracts/`. The CLI advertises the path ([relayer_cli.rs:60](../../chain-registry/crates/cli/src/relayer_cli.rs#L60)). The real, `alloy`-based bridge anchoring already lives in [node/src/bridge.rs:20](../../chain-registry/crates/node/src/bridge.rs#L20).
- **Impact:** Operators running `creg relayer soak --live` get believable but false anchoring evidence.
- **Remediation:** Delete `L1RelayerService` and `CheckpointRelayQueue`'s live path. Move `merkle_proof` helpers used by stress tests into `common` test-support. Retire `crates/cross-chain`. All anchoring goes through `node::bridge`.

#### `ECA-27` [MEDIUM] `ethers 2.0.14` is a dead dependency that carries 11 advisory exceptions

- **Evidence:** `crates/cross-chain/src` contains zero `ethers`, `reqwest` or `alloy` references. The `ethers = { version = "2", … }` line ([cross-chain/Cargo.toml:10](../../chain-registry/crates/cross-chain/Cargo.toml#L10)) alone pulls `reqwest 0.11`, `h2 0.3`, `rustls 0.21`, `rustls-webpki 0.101`, `webpki-roots 0.25`, `ring 0.16` and `rustls-pemfile 1.0`: 11 `deny.toml` ignores ([deny.toml:55-65](../../chain-registry/deny.toml#L55-L65), [:82](../../chain-registry/deny.toml#L82)). It also shares `fxhash`/`instant` with `sled` ([:75-76](../../chain-registry/deny.toml#L75-L76)).
- **Remediation:** Remove the dependency as part of `ECA-26`. **No alloy port is needed**, because no ethers code exists to port.

#### `ECA-28` [HIGH] Hot-wallet EOA nonce race in the relayer and faucet (needs load repro)

- **Evidence:** Each sponsored transaction builds a fresh `ProviderBuilder::new().with_recommended_fillers().wallet(...)` ([relayer main.rs:1562-1588](../../chain-registry/crates/relayer/src/main.rs#L1562-L1588)). The faucet does the same per drip ([faucet main.rs:2853-2950](../../chain-registry/crates/faucet/src/main.rs#L2853-L2950)). The nonce filler asks the RPC for the pending nonce per call, and nothing serialises sends across requests. The relayer's sled journal serialises the *owner's* relayer nonce, not the relayer EOA's L1 nonce. The faucet's PG advisory locks are per address/IP key.
- **Impact:** Two concurrent sponsor or drip requests for different users take the same EOA nonce. One fails with `nonce too low` or `replacement underpriced`, and the relayer then parks the user's reserved nonce and quota in "held for reconciliation".
- **Remediation:** One `SignerActor` per hot key: a single Tokio task owning the wallet, a durable EOA nonce ledger (reserved → broadcast → mined/dropped), gap and replacement handling, and fee bumping. Requests submit through a bounded `mpsc`.

#### `ECA-29` [MEDIUM] Client-IP derivation is spoofable across the relayer, hub-api and nginx

- **Evidence:** The relayer takes the **leftmost** `X-Forwarded-For` entry when `trust_proxy` is set ([relayer main.rs:2003-2011](../../chain-registry/crates/relayer/src/main.rs#L2003-L2011)). hub-api trusts the leftmost XFF unconditionally ([rateLimit.ts:16-22](../../chain-registry/hub-api/src/middleware/rateLimit.ts#L16-L22)). Both nginx front-ends append with `$proxy_add_x_forwarded_for`, preserving the client-supplied prefix ([hub-web/nginx.conf:16](../../chain-registry/hub-web/nginx.conf#L16), [explorer/nginx.conf:95](../../chain-registry/explorer/nginx.conf#L95)). There is no IPv6 `/64` bucketing.
- **Remediation:** Configure a trusted hop count and take the entry `N` from the **right**. Set `X-Real-IP $remote_addr` at the first proxy and have downstream services read only that. Bucket IPv6 by `/64`.

#### `ECA-30` [MEDIUM] Single L1 RPC endpoint, and receipt tracking gives up

- **Evidence:** The relayer reads one `RELAYER_RPC_URL` ([relayer main.rs:87](../../chain-registry/crates/relayer/src/main.rs#L87)). `watch_request_receipt` polls 90 × 2 s and then records `timed_out` with no drop, replace or reorg reconciliation ([:1590-1640](../../chain-registry/crates/relayer/src/main.rs#L1590-L1640)). The workspace already has a quorum-reading, fail-closed transport: `common::l1_rpc::{parse_rpc_endpoints, build_provider_from_urls, FailoverTransport}` ([common/src/l1_rpc.rs:96-130](../../chain-registry/crates/common/src/l1_rpc.rs#L96-L130), [:314-372](../../chain-registry/crates/common/src/l1_rpc.rs#L314-L372)). The faucet uses endpoint lists; the relayer does not use it.
- **Remediation:** The relayer adopts `common::l1_rpc` (self-hosted endpoints listed first) for reads and as the signer's transport. Confirmation-depth finality applies. The receipt reconciler runs until the transaction is mined, dropped (nonce passed) or replaced.

#### `ECA-31` [MEDIUM] The faucet PoW challenge store enables memory and CPU amplification

- **Evidence:** Every unauthenticated `GET /api/challenge` inserts into an unbounded `DashMap` and runs an O(n) `retain` over all live challenges ([faucet main.rs:3034-3052](../../chain-registry/crates/faucet/src/main.rs#L3034-L3052)). Difficulty is fixed at 20 bits.
- **Remediation:** Stateless challenges, `challenge = HMAC(k, ts ‖ rand ‖ client_ip_bucket)`, verified on redeem with single-use enforced in the existing PG claim transaction. Add a per-IP challenge rate. Make difficulty adaptive.

#### `ECA-32` [MEDIUM] Secret and transport hygiene in the relayer and faucet

- **Evidence:** Hot keys are held as plain `String` in config structs ([relayer main.rs:59-62](../../chain-registry/crates/relayer/src/main.rs#L59-L62)) and re-parsed per request. The faucet's `FAUCET_ADMIN_TOKEN` is read straight from the environment rather than through `chain-registry-secrets` ([faucet main.rs:3966](../../chain-registry/crates/faucet/src/main.rs#L3966)). The PG cooldown ledger connects with `NoTls` ([:270](../../chain-registry/crates/faucet/src/main.rs#L270)). Admin routes share the public `0.0.0.0` listener with `CorsLayer` `Any` ([:2710-2779](../../chain-registry/crates/faucet/src/main.rs#L2710-L2779)). The relayer defaults to CORS `Any` ([relayer main.rs:1296-1309](../../chain-registry/crates/relayer/src/main.rs#L1296-L1309)). One `Arc<Mutex<tokio_postgres::Client>>` serialises all claims ([faucet main.rs:261](../../chain-registry/crates/faucet/src/main.rs#L261)).
- **Remediation:** Hold keys as `Zeroizing` inside the `SignerActor` only. Route all tokens through `chain-registry-secrets`. Require TLS to PG (or a UNIX socket). Put the admin surface on a separate loopback listener. Production CORS must be an allowlist and startup refuses otherwise. Use a PG pool (`deadpool`/`sqlx`).

#### `ECA-33` [MEDIUM] `sled`, retired `zk-validator`, and permissive `deny.toml` bans

- **Evidence:** `sled 0.34.7` is used by the relayer journal ([relayer main.rs:1367-1500](../../chain-registry/crates/relayer/src/main.rs#L1367-L1500)) and the resolver cache, and declared but unused in `db-sync` ([db-sync/Cargo.toml:15](../../chain-registry/crates/db-sync/Cargo.toml#L15)). `zk-validator` is linked only by the CLI and carries 2 ignores ([deny.toml:67-69](../../chain-registry/deny.toml#L67-L69)). `[bans] multiple-versions = "warn"`, `wildcards = "allow"`, and `[licenses]` is unchecked ([deny.toml:85-90](../../chain-registry/deny.toml#L85-L90)).
- **Remediation:** Relayer journal → PostgreSQL (the faucet's durable-ledger pattern via `sqlx`); resolver → `ECA-15`; db-sync → delete the dependency. Remove `zk-validator` from the CLI and decide whether to delete the crate. Tighten `wildcards = "deny"`, add a `[licenses]` allowlist, and reach 15 ignores removed (§10).

---

## 9. Group E — Explorer, Hub & Public Node API

#### `ECA-34` [HIGH] The explorer misrenders revocations and quarantine as success

- **Evidence:** `Transaction` serialises as `type: "revoke_v2"` ([block.rs:241](../../chain-registry/crates/common/src/block.rs#L241)), but the explorer only recognises `type === 'revoke'` ([normalize.js:21](../../chain-registry/explorer/src/api/normalize.js#L21), [BlockDetail.jsx:172](../../chain-registry/explorer/src/pages/BlockDetail.jsx#L172), [TxDetail.jsx:111](../../chain-registry/explorer/src/pages/TxDetail.jsx#L111)). Every revocation the current node produces (`RevokeV2`, `ECA-01`) is therefore shown as a green **finalized** transaction. `ProofVerifier` shows a success badge for any `packageStatus !== 'revoked'`, including `quarantined` ([ProofVerifier.jsx:134](../../chain-registry/explorer/src/pages/ProofVerifier.jsx#L134)). `PackageDetail` uses a non-existent `'neutral'` badge variant ([PackageDetail.jsx:36](../../chain-registry/explorer/src/pages/PackageDetail.jsx#L36)). `explorer`, `hub-api` and `hub-web` sources contain zero references to quarantine, witnesses, model families or bounties.
- **Remediation:** Map every status and transaction type through the generated `registry-view` enums, so an unknown value is a TypeScript error and renders as a neutral `UNKNOWN`. Add a revocation and quarantine timeline.

#### `ECA-35` [HIGH] No CSP or HSTS anywhere, and nginx header inheritance strips the explorer document's headers

- **Evidence:** A repo-wide search finds no `Content-Security-Policy` or `Strict-Transport-Security`. The explorer sets security headers at server level ([explorer/nginx.conf:8-12](../../chain-registry/explorer/nginx.conf#L8-L12)). But `location = /index.html` declares its own `add_header Cache-Control` ([:150-153](../../chain-registry/explorer/nginx.conf#L150-L153)), and nginx inherits `add_header` only when the current level defines none. The SPA fallback `try_files $uri $uri/ /index.html` lands in that location, so the HTML document ships **without** `X-Frame-Options`, `nosniff` or `Referrer-Policy`. Static and spec locations behave the same way. `hub-web/nginx.conf` sets no security headers at all.
- **Impact:** Clickjacking of wallet and staking flows, and no script-execution backstop for the planned rendering of attacker-influenced witness and AI telemetry.
- **Remediation:** Shared `security-headers.conf` included in **every** location: CSP `default-src 'self'; script-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; connect-src 'self' <wallet RPC origins>; require-trusted-types-for 'script'`, plus HSTS, `nosniff`, `Referrer-Policy`, `Permissions-Policy` and COOP. Add a Playwright test that asserts headers on `/`, a deep link, and an asset.

#### `ECA-36` [HIGH] The hub-api rate limiter is bypassable, leaks memory, and amplifies upstream load

- **Evidence:** Leftmost-XFF keying (`ECA-29`). The module-global `buckets` Map is never pruned ([rateLimit.ts:8](../../chain-registry/hub-api/src/middleware/rateLimit.ts#L8)), so random XFF values grow it without limit. Without proxy headers, all clients share the `"unknown"` bucket, so 20 auth requests a minute globally lock out sign-in. `/api/status/public` fans out six uncached upstream fetches per request ([index.ts:150-170](../../chain-registry/hub-api/src/index.ts#L150-L170)).
- **Remediation:** A bounded LRU limiter keyed by the trusted client IP. Cache the status aggregate for 5–10 s with single-flight. Fail closed with 503 when the client IP cannot be established in production.

#### `ECA-37` [MEDIUM] Real-time and polling scalability

- **Evidence:** Node SSE and WS have no subscriber cap and no event `id:`, so there is no `Last-Event-ID` resume. The WS handler never reads client frames and disconnects silently on lag ([events.rs:211-237](../../chain-registry/crates/node/src/events.rs#L211-L237), [:245-281](../../chain-registry/crates/node/src/events.rs#L245-L281)). The explorer polls each widget every 5 s ([usePolling.js:7](../../chain-registry/explorer/src/hooks/usePolling.js#L7)) and subscribes to only six legacy kinds ([useSse.js:26-27](../../chain-registry/explorer/src/hooks/useSse.js#L26-L27)).
- **Remediation:** SSE v2 on the gateway with a global and per-IP subscriber cap, `id: <height>.<index>`, a replay buffer from the indexer for resume, and typed v4 kinds. The explorer becomes SSE-first, with polling only as a degraded fallback.

#### `ECA-38` [MEDIUM] The explorer `/rpc` route is an open proxy to the operator's Sepolia RPC

- **Evidence:** [explorer/nginx.conf:129-138](../../chain-registry/explorer/nginx.conf#L129-L138) has no JSON-RPC method allowlist, no rate limit and no body cap.
- **Remediation:** Point at a self-hosted RPC. Gate with a small method allowlist (`eth_chainId`, `eth_call`, `eth_getTransactionReceipt`, `eth_blockNumber`, `eth_estimateGas`), `limit_req`, and a 64 KiB body cap. Wallet sends go through the user's own wallet provider.

#### `ECA-39` [MEDIUM] Public metrics and docs exposure fails open

- **Evidence:** `expose_public_metrics_docs` is read with `state.try_read().map(...).unwrap_or(true)` ([api.rs:111-114](../../chain-registry/crates/node/src/api.rs#L111-L114)). If the state lock is contended while the router is built, `/metrics` and Swagger are mounted publicly.
- **Remediation:** Pass the flag as a plain `bool` from `NodeConfig` into `router()`, defaulting to `false`.

#### `ECA-40` [LOW] SIWE nonce consumption is not atomic, and session hygiene is weak

- **Evidence:** The nonce is checked, then `await siweMessage.verify(...)`, then `UPDATE nonces SET consumed_at` ([auth.ts:136-172](../../chain-registry/hub-api/src/routes/auth.ts#L136-L172)), so parallel replays of one signed message mint several sessions. Verification error detail is returned to the client. Sessions are only garbage-collected when read. Cookies are `Secure` only when `NODE_ENV=production` ([index.ts:33-34](../../chain-registry/hub-api/src/index.ts#L33-L34)).
- **Remediation:** `UPDATE … SET consumed_at=? WHERE nonce=? AND consumed_at IS NULL` before verify, requiring `changes === 1`. Return a generic error. Run a periodic session purge. Make `Secure` the default.

#### `ECA-41` [LOW, latent] The hub-web markdown renderer allows `javascript:` links into `innerHTML`

- **Evidence:** `inline()` turns `[x](javascript:…)` into `<a href="javascript:…">` ([markdown.ts:38](../../chain-registry/hub-web/src/lib/markdown.ts#L38)), rendered via `dangerouslySetInnerHTML` ([MarkdownContent.tsx:11](../../chain-registry/hub-web/src/components/MarkdownContent.tsx#L11)). `MarkdownContent` is currently unused, but it is the obvious sink a future README or AI-report panel would reuse.
- **Remediation:** Allowlist `https:` and `mailto:`, or delete the component and render markdown with a sanitizing library under Trusted Types.

#### `ECA-42` [MEDIUM] `DiscoveryBounty` has an unpaid committee share and is L1-only

- **Evidence:** `distributeDiscoveryBounty` credits and transfers only `proverBonus` (25%). `committeeShare` (75%) is computed and emitted but never credited, reserved or transferred ([ValidatorRewards.sol:374-387](../../chain-registry/contracts/ValidatorRewards.sol#L374-L387)). The event exists only on L1, while `db-sync` reads only the CREG chain. `witnessDigest` has no on-chain link to a CREG `RevokeV2`.
- **Impact:** A "25% prover / 75% committee" leaderboard would display money that is never paid. The indexer has no ingest path for the event.
- **Remediation:** Contract fix (credit per-member committee shares, or explicitly reserve the amount in a `committeeBountyPool`), plus an L1 log-ingest lane in the indexer joined to `RevokeV2.witness` digests. The UI labels committee share as "reserved" until the contract fix ships.

---

## 10. `deny.toml` Burn-Down Ledger

Baseline: **50** advisory ignores ([deny.toml:19-83](../../chain-registry/deny.toml#L19-L83)).

| Removal trigger | Advisory IDs removable | Count |
| --- | --- | :---: |
| Delete `ethers` from `crates/cross-chain` (`ECA-26`/`ECA-27`) | `RUSTSEC-2026-0258` (h2 0.3), `RUSTSEC-2025-0009`, `RUSTSEC-2025-0010` (ring 0.16), `RUSTSEC-2026-0346`, `RUSTSEC-2026-0349` (rustls 0.21), `RUSTSEC-2026-0098`, `-0099`, `-0104`, `-0382` (rustls-webpki 0.101), `RUSTSEC-2026-0381` (webpki-roots 0.25), `RUSTSEC-2025-0134` (rustls-pemfile 1.0) | 11 |
| Delete `ethers` **and** retire `sled` from resolver, relayer and db-sync (`ECA-15`, `ECA-33`) | `RUSTSEC-2025-0057` (fxhash), `RUSTSEC-2024-0384` (instant / parking_lot 0.11) | 2 |
| Unlink `zk-validator` from all built crates (`ECA-16`, `ECA-33`) | `RUSTSEC-2025-0055` (tracing-subscriber 0.2), `RUSTSEC-2024-0388` (derivative) | 2 |
| **Total removable** | | **15** |
| Remaining, upstream-blocked | libp2p/hickory (18), `rsa`/`crypto-bigint` crypto pins (2), utoipa build-only `zip 1.3` (7), transitive unmaintained (`bincode`, `chrono`, `core2`, `number_prefix`, `paste`, `pin-utils`, `proc-macro-error`, `proc-macro-error2`) (8) | 35 |

`number_prefix` (`RUSTSEC-2025-0119`) can also go if `indicatif` is upgraded past 0.17 in the CLI. Each removal must be confirmed with `cargo deny --workspace check advisories bans sources` after the code change, because a transitive path may survive.

---

## 11. Findings Register

| ID | Sev | Component | Summary | ADR-019 Inc |
| --- | :---: | --- | --- | :---: |
| `ECA-01` | CRIT | node/consensus | v4 artifacts never produced | 2 |
| `ECA-02` | CRIT | node/resolver | Single-signer `RevokeV2`, permanent client block | 2 / 3 |
| `ECA-03` | HIGH | node/explorer | Lane C LLM in consensus process; public LLM prose | 2 / 7 |
| `ECA-04` | HIGH | zone-ipc | Shared symmetric token; no replay/peer-cred | 4 |
| `ECA-05` | HIGH | node API | No v4 read contract (OpenAPI/DTO/SSE) | 2 |
| `ECA-06` | MED | common | Open-ended v4 display vocabulary | 2 |
| `ECA-07` | CRIT | resolver | Quarantined installable via break-glass | 1 |
| `ECA-08` | HIGH | resolver | Proof verification compile-gated off in `creg` | 3 |
| `ECA-09` | HIGH | resolver | Ed25519-only PBFT check; no activation height | 3 |
| `ECA-10` | HIGH | resolver/node | No current-status / non-revocation proof | 2 / 3 |
| `ECA-11` | HIGH | resolver | gRPC version confusion + TLS downgrade | 1 |
| `ECA-12` | HIGH | resolver | Freshness checks inert | 3 |
| `ECA-13` | HIGH | resolver/cli | 24h unauthenticated cache drives audit/verify | 1 |
| `ECA-14` | MED | resolver/cli | Unbounded downloader, predictable temp path | 1 |
| `ECA-15` | MED | resolver | Cache/journal crash safety; `sled` | 3 |
| `ECA-16` | HIGH | cli | CLI links validator/zk/ml/wasm/cross-chain | 4 |
| `ECA-17` | MED | cli | Terminal escape injection | 1 |
| `ECA-18` | MED | cli | Unbounded SSE buffer; CI blind to v4 | 1 / 4 |
| `ECA-19` | MED | cli | Secrets on argv; `f64` amounts | 1 |
| `ECA-20` | MED | cli | Signature-derived publisher key; wildcard loopback CORS | 4 |
| `ECA-21` | LOW | cli | No zone/sandbox/AI operator diagnostics | 4 |
| `ECA-22` | HIGH | db-sync | Schema blind to v4 & revocation provenance | 5 |
| `ECA-23` | HIGH | db-sync | `stake_wei BIGINT` overflow | 1 |
| `ECA-24` | MED | db-sync | Schema drift, no migrations, cursor outside tx | 5 |
| `ECA-25` | MED | db-sync/node | In-process indexer; per-block RocksDB reopen | 5 |
| `ECA-26` | HIGH | cross-chain | Fabricated "live" L1 relay receipts | 1 / 6 |
| `ECA-27` | MED | cross-chain | Dead `ethers` dependency (11 ignores) | 6 |
| `ECA-28` | HIGH | relayer/faucet | Hot-wallet EOA nonce race | 6 |
| `ECA-29` | MED | relayer/hub/nginx | Spoofable client IP | 1 |
| `ECA-30` | MED | relayer | Single RPC; receipt watcher gives up | 6 |
| `ECA-31` | MED | faucet | PoW challenge store DoS | 6 |
| `ECA-32` | MED | relayer/faucet | Secret/transport hygiene | 6 |
| `ECA-33` | MED | workspace | `sled`, `zk-validator`, permissive bans | 6 |
| `ECA-34` | HIGH | explorer | `RevokeV2`/quarantine rendered as success | 1 / 7 |
| `ECA-35` | HIGH | explorer/hub-web | No CSP/HSTS; header inheritance bug | 1 |
| `ECA-36` | HIGH | hub-api | Limiter bypass, memory leak, amplification | 1 |
| `ECA-37` | MED | node/explorer | SSE/WS caps, no resume, polling | 7 |
| `ECA-38` | MED | explorer | Open `/rpc` proxy | 1 |
| `ECA-39` | MED | node API | Metrics/docs fail-open | 1 |
| `ECA-40` | LOW | hub-api | SIWE nonce race; session hygiene | 7 |
| `ECA-41` | LOW | hub-web | Latent `javascript:` markdown sink | 7 |
| `ECA-42` | MED | contracts/indexer | Unpaid committee bounty share; L1-only event | 5 / 7 |

**Totals:** 3 CRITICAL · 17 HIGH · 19 MEDIUM · 3 LOW (42 findings).
