# Next work — prioritized backlog

**Updated:** 2026-08-24  
**HEAD:** `9ebfa8c` (PR #51; Track B recruiting in flight)  
**Progress briefing:** [PROTOCOL_PROGRESS.md](./PROTOCOL_PROGRESS.md)  
**Planning hub:** [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md)  
**Context:** [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md), [WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md), [TESTNET_STATUS.md](./TESTNET_STATUS.md)

---

## Current phase

- **In-repo protocol through PR #50:** GOVERNANCE-API, SHIM-301, npm
  direct-dep gate, CLIENT-FRESHNESS, and SUPPLY-CHAIN-CI are on `main`.
- **WITNESSED-LOG Track B started (2026-08-24):** recruitment only —
  [WITNESS_OPERATOR_ONBOARDING.md](./WITNESS_OPERATOR_ONBOARDING.md),
  [WITNESS_KEY_CEREMONY.md](./WITNESS_KEY_CEREMONY.md), empty
  `testnet/witnesses/roster.json`. `creg keygen witness` / `log-signer`
  write keys outside the git worktree. This does **not** close the ID.
  Do not mint local stand-ins. Public testnet resume is still
  operator work (VAL-INDEPENDENCE, then TESTNET-RESUME).
- **Protocol hardening:** CR-07 namespace authority and CR-04/HI-11
  validator-history protections are implemented at repository level but remain
  disabled pending trust material, a real checkpoint, and independent
  operational evidence.
- **Verification:** focused node, full node (225/225 lib; 227/227 `creg-node`),
  CLI (49/49), node E2E (8/8), node integration (11/11), workspace all-target
  at `5868401` (exit 0, including sidecar 2/2), `cargo fmt --all -- --check`,
  and pinned Foundry v1.7.1 `forge test` (129/129) pass locally. Remote GitHub
  Actions is deferred until the billing period resets; do not raise the
  spending limit. PDF regeneration is blocked in this checkout (Arena pipeline
  absent); do not commit `docs/document-pdfs/`.
- **Network posture:** `chain-spec.sepolia.json` lists four coordinated
  validators (`validator-1`…`validator-4`) and empty bootnodes. That is the
  specified public-alpha topology, not a live multi-operator network and not
  Byzantine fault tolerance. The public GCP fleet is paused; see
  [WAITLIST-ONLY-HOSTING.md](./WAITLIST-ONLY-HOSTING.md) and
  [TESTNET_STATUS.md](./TESTNET_STATUS.md).
- **Out of this workstream:** Contribution Arena is a separate educational
  and simulation product. Protocol planning here does not include Arena
  launch, findings routing, or Hub cutover.
- **Activation posture:** do not enable authoritative recovery or mandatory
  certified committees until the signed activation/checkpoint ceremonies and
  multi-operator rehearsals are complete. ISSUE-014/018 are in-repo fail-closed
  patches; they do not flip **HI11-ACTIVATE** or **CR07-ACTIVATE**.

---

## P0 — Gating work

| # | ID | Status | Task | Acceptance |
|---|-----|--------|------|------------|
| 1 | **VERIFY-401** | locally complete; remote CI deferred | Finish reproducible repository verification | Local: workspace all-target at `5868401` (exit 0), rustfmt `--check`, Foundry v1.7.1 `forge test` 129/129. Remote Actions waits for the next GitHub billing period; do not increase the spending limit. |
| 2 | **CR07-ACTIVATE** | blocked externally | Production namespace-authority package | Reviewed npm/PyPI policy and trust roots, official npm authentication-mode signal, positive fixtures, signed manifest, platform artifacts, and multi-operator rehearsal. |
| 3 | **HI11-ACTIVATE** | blocked externally | Certified-history activation | Independently verified checkpoint plus replay, partition, reorg, rebuilt-node, long-range, and process-kill rehearsals before mandatory mode. |
| 4 | **VAL-INDEPENDENCE** | blocked externally | Independent validators and bootnodes | Recruit and verify separately controlled operators/keys/infrastructure; a four-validator coordinated spec must not be represented as Byzantine-fault-tolerant. |
| 5 | **SPEC-ROTATE** | superseded | Repair signed Sepolia specification | Fresh-genesis candidate recorded 2026-08-01/02 in `chain-spec.sepolia.json` (genesis 2026-08-02). Remaining work is independent verification and public activation, not another in-repo rotate. |
| 6 | **SEC-401** | open | External security audit | Scope: [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md). Candidate freeze: `v0.4.3-testnet` (`3eb3a23…`). Vendor, start date, and signed findings remain TBD. Cut a later tag only if protocol diffs land after VERIFY-401. |
| 7 | **DOC-RELEASE** | blocked in this checkout | Publish synchronized protocol documents | Public README/PUBLIC/install examples pin `v0.4.3-testnet`. PDF regen lives in the Contribution Arena `generate:document-pdfs` pipeline, which is **not present** in this workspace (no Arena `package.json`). Untracked `docs/document-pdfs/` must not be committed. Do not treat Arena launch as protocol work. |
| 8 | **TESTNET-RESUME** | gated | Resume public fleet only after operator gates | Use `testnet/gcp/resume-testnet-hosting.ps1` and `hosting-301-verify.ps1` only after independent validators, checkpoint, trust, and rehearsal sign-offs. |

---

## P1 — Quality (next 2–4 weeks)

| # | ID | Status | Task | Acceptance |
|---|-----|--------|------|------------|
| 8 | **EIP712-401** | in-repo complete; review/redeploy open | Epoch-bound contract signatures | Solidity `PackageVerdict` / Staking admission bind `validatorSetEpoch`. Rust `consensus_admission.rs` fetches the live epoch, signs it, and aborts the tick on RPC failure or mid-tick epoch change. Local Foundry v1.7.1 129/129. Remaining: independent contract review, Sepolia ABI/bytecode upgrade, remote Contract CI after billing reset. |
| 9 | **ARCHIVE-401** | repository complete; ops open | Authority evidence resource controls | Exercise pressure GC/restore; add capacity alerts, shared-filesystem coordination decision, backup, and legal-hold policy. |
| 10 | **BRIDGE-301** | open | L1 bridge soak | `CREG_BRIDGE_KEY` on validator; batch observed or documented skip + alert. |
| 11 | **SOAK-301** | open | Incentive/load soak | Run [testnet/SOAK_TEST.md](../chain-registry/testnet/SOAK_TEST.md) and availability/DR drills. |
| 12 | **SEC-402** | open | Network partition chaos | Run `k8s/55-network-partition-test.yaml`, collect traces, and publish a postmortem. |
| 13 | **ISSUE-013** | done (2026-08-22) | Fail-closed sandbox bypass | Merged in PR #26: crate-level bypass requires `CREG_TESTNET` and blocks public/production profiles. |

---

## P2 — Soon (not blocking)

| # | ID | Task |
|---|-----|------|
| 14 | **WITNESSED-LOG** | Track B recruiting (roster empty). Replace legacy/local history authority with a witnessed append-only log. Independent witness operators, signed-head distribution, and the Track A daemon remain. |
| 18 | **REM-211+** | Complete resource metrics and Grafana dashboards per [observability/README.md](../chain-registry/observability/README.md). |
| 19 | **VAL-SCALE** | Expand a verified independent validator cohort in capacity-tested batches; waitlist size is not validator capacity. |
| 20 | **ZK-CIRCUIT** | Complete circuit/contract review and regenerate setup only if the binding changes; do not restore the retired qualification claim. |

---

## Completed (reference)

| ID | Done | Notes |
|----|------|-------|
| NET-301 | 2026-06-09 | 2-validator PBFT quorum |
| SANDBOX-301 | 2026-06-10 | Real nsjail sandbox |
| DIST-301 | 2026-06-10 | `v0.1.0-testnet` / `v0.1.2-testnet` binaries |
| DIST-401 | 2026-08-03 | `v0.4.3-testnet` binaries (current public pin) |
| HOSTING-301 | 2026-06-10 | `testnet.cregnet.dev` (resumable; currently paused) |
| SPEC-ROTATE | 2026-08-02 | Fresh genesis candidate in `chain-spec.sepolia.json`; public activation still gated |
| DOC-301 | 2026-06-08 | Doc index rationalization |
| GAPS-Iter5 | 2026-06-26 | Gossip/PBFT hardening |
| GAPS-Iter6 | 2026-06-26 | Batch + finalize relay forge tests |
| HI11-REPO | 2026-07-29 | Certified transitions, gossip validation, checkpoint ancestry, mandatory gate, reorg/retention tests; live activation still pending |
| CR07-ARCHIVE | 2026-07-29 | Bounded evidence/artifact archive with minimum-retention pressure GC; operational drills still pending |
| ISSUE-013 | 2026-08-22 | `CREG_DEV_SANDBOX` requires testnet profile; public/production blocked (PR #26) |
| ISSUE-010 | 2026-08-22 | CrossChain `AxelarAdapter.setChainName` is governance-gated; adapter remains undeployed |
| ISSUE-015 | 2026-08-22 | WASM WASI stubs gated by `CapabilitySet` / `ResourceLimits`; stdout capture via `fd_write` |
| ISSUE-011 | 2026-08-23 | Publisher ZK admission placeholders retired; gRPC rejects leftover 85/`true` proofs |
| ISSUE-016 | 2026-08-23 | Historical sync verifies `validator_set_hash` against the set at that height; HI-11 checkpoint still operational |
| ISSUE-017 | 2026-08-23 | `GET /v1/governance/proposals` mirrors Governance.sol; 200 empty if unconfigured, 503 on L1 RPC failure |
| VAL-JOIN | 2026-08-23 | Independent validators can join PBFT/P2P in-repo (PR #41); recruiting operators remains VAL-INDEPENDENCE |
| OPERATOR-PACK | 2026-08-23 | Validator compose, k8s, fail-closed YARA votes (PR #42) |
| SPEC-OVERRIDE | 2026-08-23 | Node refuses env contract addresses that disagree with a signed spec (PR #44); compose defaults aligned (PR #45) |
| GOVERNANCE-API | 2026-08-23 | Operator-authenticated submit/vote/execute; `CREG_GOVERNANCE_KEY` required; public POST stays 405 |
| SHIM-301 | 2026-08-23 | PATH shims share `creg install` fail-closed gate; 5-minute install cache; `CREG_ALLOW_UNVERIFIED` break-glass; bare `npm install`/`ci` gates direct package.json deps |
| CLIENT-FRESHNESS | 2026-08-23 | Install/shim anti-rollback journal, freeze/skew rejection, and opt-in witnessed inclusion (`verify_witnessed_inclusion`). Unsigned transparency heads stay non-install-eligible. Independent witness ops remain under WITNESSED-LOG. |
| SUPPLY-CHAIN-CI | 2026-08-23 | Workspace `Cargo.lock` SPDX/CycloneDX inventory, `creg supply-chain check` deny policy, lockfile-bound provenance (explicitly not SLSA). Package `creg sbom` marks incomplete without declared deps. Submitted-package SLSA and web/E2E remain. |
| ISSUE-012 | 2026-08-23 | Deep scan fails closed on corrupt/timeout input; votes carry the actual `degraded-*` scanner version |
| ISSUE-014 | 2026-08-23 | Spec `threshold_encryption` now fail-closes; Sepolia flag stays false until share collection ships |
| ISSUE-018 | 2026-08-23 | Appeal publisher/panelist payouts are pull-payment; not yet redeployed on Sepolia |
| ISSUE-024 | 2026-08-23 | Validators refuse missing/invalid `CREG_STAKING_ADDR`; no silent static-set fallback |
| ISSUE-019 | 2026-08-23 | Multisig submit uses `publisher_packages_url` (`/v1/publisher/packages`) |
| ISSUE-020 | 2026-08-23 | Install policy evaluates REST/gRPC-overlay publisher; empty identity fail-closes on allow-list |
| ISSUE-023 | 2026-08-23 | CLI advanced sandbox miss is UNSAFE (`sandbox_declared_safe(Err) -> false`) |
| ISSUE-004 | 2026-08-23 | Malformed `CREG_ETH_RPC` is a hard startup error; bridge no longer panics on parse |
| ISSUE-025 | 2026-08-23 | Shamir `gf_div` errors on zero divisor; reconstruction rejects dup/index-0 shares |
| ISSUE-021 | 2026-08-23 | Publisher rotation nonce/cooldown uses `publisher_rotation` CF with lazy backfill |
| ISSUE-022 | 2026-08-23 | Double-sign evidence encodes Ed25519 R\|\|S; private key stays redacted |
| ISSUE-005 | 2026-08-23 | Production missing ZK keys return Err and fail startup; Sepolia zk_validation stays false |
| EIP712-401-RUST | 2026-08-23 | Admission signer binds live `validatorSetEpoch` and fail-closes on RPC/epoch drift |
| DOC-RELEASE | blocked here | Arena `generate:document-pdfs` pipeline is not in this checkout; do not commit `docs/document-pdfs/` |
| VERIFY-401-RUST | 2026-08-23 | Workspace all-target Cargo run exit 0 at `b7fabe5` and again at `5868401` after PR #36 |
| VERIFY-401-SIDECAR | 2026-08-23 | Private-temp launch: host-visible TEMP after env_clear; fixture + local frozen Windows rejection |
| VERIFY-401-FMT | 2026-08-23 | `cargo fmt --all -- --check` exit 0 at `5868401` (local rustc 1.93.1; CI pin is 1.93.0) |
| VERIFY-401-FOUNDRY | 2026-08-23 | Pinned Foundry v1.7.1 `forge test` 129/129 locally; not the profile 1.5.1 binary |
| VERIFY-401-CI | deferred | GitHub Actions billing/spending; retry after the next monthly reset. Do not raise the limit. |

---

## Quick commands

```powershell
cd chain-registry

# Local verification (run from chain-registry/)
cargo fmt --all -- --check
cargo test --locked --workspace --all-targets -j1
forge test

# Maintainer seed fleet env
.\testnet\init-sepolia-fleet-env.ps1

# Resume public hosting (when ready)
.\testnet\gcp\resume-testnet-hosting.ps1
.\testnet\hosting-301-verify.ps1 -BaseDomain testnet.cregnet.dev

# Stop public hosting (cost saving)
.\testnet\gcp\stop-testnet-hosting.ps1
```

---

_Update this file when an item ships; mirror in [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md)._
