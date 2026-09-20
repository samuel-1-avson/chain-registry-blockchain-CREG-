# CREG protocol progress

**As of:** 2026-09-20
**HEAD:** `9173c37` (merge; observer framing, signed-spec Registry, truthful Arena docs).
**Public binary pin:** `v0.4.8-testnet` provides Windows, Linux, and macOS archives through the GitHub release. SEC-401 freeze candidate can stay `v0.4.3-testnet` if a vendor already scoped that tag.
**Canonical repo:** [samuel-1-avson/chain-registry-blockchain-CREG-](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-)

This page is the **session briefing** for protocol work: what shipped, what improved, what is left, and how ready CREG is for testnet. Historical fleet facts: [TESTNET_STATUS.md](./archive/TESTNET_STATUS.md) (2026-08-25 snapshot). Archived tracks: [NEXT_WORK.md](./archive/NEXT_WORK.md), [WHAT_TO_DO_NEXT.md](./archive/WHAT_TO_DO_NEXT.md).

---

## Testnet readiness (honest)

| Audience | Ready? | Meaning |
|----------|--------|---------|
| Maintainer local node | **Yes** | Build and run `creg-node` / `creg` against the signed Sepolia spec. |
| Self-hosted operator vs spec | **Partial** | You can run a node; there is no public bootnode list to join. |
| Publisher on `api.testnet.cregnet.dev` | **No** | API reachable but serves a fresh empty chain (tip 0, no peers, no validators, observed 2026-09-20); not a live network. |
| Independent BFT public testnet | **No** | Spec lists four non-voting observer identities and two bootnodes on one GCP host. Not BFT. |
| Mainnet / economic security | **No** | Public alpha only. ZK qualification stays off. No signed audit findings. |

**Launch-level shorthand:** software is a coordinated public-alpha stack (past lab-only). There is **no live public network**: the API serves an empty chain with no peers (2026-09-20). Do not describe CREG as a live public testnet, as Byzantine-fault-tolerant, or as mainnet.

**What “ready for testnet” is not:** more in-repo features. Remaining public-testnet gates are operators, ceremonies, billing, audit vendor, and PDF regeneration.

---

## What shipped (22–23 August 2026)

In-repo fail-closed hardening on `main` through **PRs 25–52**. Remote GitHub Actions still does not start (account billing / spending limit). Do not raise the spending limit. Do not treat failed Actions as a code regression.

### WITNESSED-LOG Track A software (this change)

`creg witness run` polls `GET /v1/public/transparency/:log_id/signed-head`, refuses unsigned `/head`, persist-and-fail-closes on split-view/rollback, and writes `WitnessedTreeHead` JSON. The log node signs that endpoint when `CREG_TRANSPARENCY_LOG_KEY` is set (otherwise 503). Clients merge independent JSON URLs via `CREG_WITNESS_HEAD_URLS`. This does **not** close WITNESSED-LOG: roster is empty, and a split-view **drill** is still required.

### This workstream (PRs 46–52)

| PR | ID | What landed | Must not claim |
|----|----|-------------|----------------|
| [#46](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/46) | GOVERNANCE-API | Operator-authenticated submit / vote / execute on `/v1/operator/governance/...`. Requires operator key + `CREG_GOVERNANCE_KEY`. Public POST stays 405. Bridge key cannot submit or vote. | Public unauthenticated governance writes |
| [#47](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/47) | SHIM-301 | PATH shims share `creg install` fail-closed gate; 5-minute install cache. `--unverified` / `CREG_ALLOW_UNVERIFIED` cannot override revocation. | Transitive npm graph is scanned |
| [#48](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/48) | npm lockfile | Bare `npm install` / `ci` gates **direct** `package.json` deps (max 200). IPFS tarball fetch remains `creg install` only. | Full lockfile / transitive audit |
| [#49](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/49) | CLIENT-FRESHNESS | Local anti-rollback journal; freeze / skew / mix-and-match rejection. `verify_witnessed_inclusion` only when log/witness keys are configured. Unsigned `GET /v1/transparency/.../head` is **not** install-eligible. | Independent witness operators (that is WITNESSED-LOG) |
| [#50](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/50) | SUPPLY-CHAIN-CI | `creg sbom --workspace` inventories `Cargo.lock` (SPDX 2.3 / CycloneDX 1.5). `creg supply-chain check` fail-closes missing checksums, non-crates.io indexes, git sources, banned names. Provenance is lockfile SHA-256. Package `creg sbom` is incomplete without declared deps. No new GitHub Actions job. | SLSA; submitted-package provenance; remote CI |
| [#51](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/51) | PROTOCOL_PROGRESS | Testnet-readiness snapshot and next-work briefing | Public testnet is live |
| [#52](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/52) | WITNESSED-LOG Track B | Empty pubkey roster, operator onboarding, `creg keygen witness` / `log-signer` (refuse keys in the git worktree) | Independent witnesses exist |

Policy file: `chain-registry/config/supply-chain/policy.toml`. Implementation: `chain-registry/crates/cli/src/supply_chain.rs`. Point HI-19 docs at [ADR-014](./adr/ADR-014-REPRODUCIBLE-BUILD-DDC.md); do not invent a gitignored supply-chain README.

### Operator / spec honesty (PRs 41–45)

| PR | What landed | Must not claim |
|----|-------------|----------------|
| [#41](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/41) | Independent validators can join PBFT and P2P (in-repo path) | Recruited independent operators exist |
| [#42](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/42) | Validator compose, k8s, fail-closed YARA votes | Live fleet is running |
| [#43](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/43) | Operator env aligned with the signed Sepolia spec | Public bootnodes |
| [#44](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/44) | Node refuses env contract addresses that disagree with a signed spec | Sepolia bytecode already matches every in-repo Solidity patch |
| [#45](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/45) | Compose defaults point at signed Sepolia contracts | Fleet resume |

### DEEP_DIVE / VERIFY pass (PRs 25–40)

Fail-closed residuals through ISSUE-005, plus local VERIFY-401 capture. Highlights: sandbox bypass requires testnet profile; WASM WASI gated; publisher ZK placeholders rejected; historical `validator_set_hash`; governance reads; threshold flag fail-closed; Appeal pull-payment in Solidity; staking address required; CLI publisher / install / sandbox residuals; production ZK keys fail-closed; workspace all-target, rustfmt, Foundry v1.7.1 recorded locally.

Full ID table: [NEXT_WORK.md](./archive/NEXT_WORK.md) completed section (archived 2026-09-19).

---

## Improvements (what is better now)

- **Admission and install path:** shims, freshness, anti-rollback, and direct-dep npm gating match `creg install` fail-closed policy more closely than before this pass.
- **Governance writes:** operator-authenticated; public POST remains 405; bridge key is not a governance key.
- **Supply chain inventory:** workspace lockfile SBOM + deny policy exist in the CLI. Explicitly **not** SLSA.
- **Spec honesty:** compose/env cannot silently override signed Sepolia contract addresses.
- **Join path:** independent validator join is implemented in-repo; recruiting operators is still external.
- **Local verification:** focused node, CLI, E2E, integration, rustfmt, and Foundry v1.7.1 `forge test` 129/129 passed locally (see [TEST_EVIDENCE.md](./remediation/TEST_EVIDENCE.md)). Extra Windows flags used: `CARGO_PROFILE_TEST_DEBUG=0`, `--test-threads=1`.

---

## What is left

### P0 — still gating a *public* testnet (mostly not in-repo)

| ID | Status | Why it is not a code flip |
|----|--------|---------------------------|
| VERIFY-401 remote CI | Deferred | GitHub Actions jobs do not start (billing). Do not raise the spending limit. Local cargo/fmt/forge are not a substitute. |
| CR07-ACTIVATE | Blocked externally | Needs reviewed npm/PyPI policy, official npm authentication-mode signal, positive fixtures, signed manifest, multi-operator rehearsal. **Do not** set `namespace_authority_activation_height`. |
| HI11-ACTIVATE | Blocked externally | Needs independently verified checkpoint plus replay / partition / reorg / rebuilt-node / long-range / process-kill rehearsals. **Do not** enable mandatory certified committees. |
| VAL-INDEPENDENCE | Blocked externally | Recruit separately controlled operators, keys, and infrastructure. Four coordinated spec IDs are not BFT. |
| TESTNET-RESUME | Gated | `resume-testnet-hosting.ps1` only after the operator gates above. API currently serves a fresh empty chain (tip 0, no peers, 2026-09-20), not a live network. |
| SEC-401 | Open / vendor TBD | Freeze candidate remains `v0.4.3-testnet`. Cut a later tag only if you want auditors on post-tag protocol diffs. |
| DOC-RELEASE | Run after doc edits | Arena `generate:document-pdfs` exists in the `CREG-Contribution-Arena` submodule; regenerate PDFs plus `sync:whitepaper` / `sync-knowledge` after doc edits. Do not commit `docs/document-pdfs/`. |

SPEC-ROTATE is **superseded** as an in-repo rotate. `chain-spec.sepolia.json` carries `genesis_time` 2026-09-19 (`0x9d9e…`); the live API serves a different empty chain (`0x0683…`). Remaining work is independent verification and public activation.

### P1 — quality (ops / L1, not another residual queue)

| ID | Status | Remaining |
|----|--------|-----------|
| EIP712-401 | In-repo complete | Independent contract review, Sepolia ABI/bytecode upgrade, remote Contract CI after billing reset. |
| ARCHIVE-401 | Repo complete; ops open | Pressure GC/restore drills, capacity alerts, backup, legal-hold. |
| BRIDGE-301 | Open | L1 bridge soak or documented skip + alert. |
| SOAK-301 | Open | Incentive/load soak after a live API exists. |
| SEC-402 | Open | Network-partition chaos + postmortem after a live API exists. |

### P2 — leftover *in-repo* protocol (do not fake)

| ID | Task | Honest bound |
|----|------|----------------|
| **WITNESSED-LOG** | Independent witness operators and signed-head distribution | Track A software is in-repo (`creg witness run`, log `signed-head`, `CREG_WITNESS_HEAD_URLS`). Track B **recruiting**: empty pubkey-only roster. Do not mint local/self-signed witnesses. Unsigned transparency heads stay non-install-eligible. Not closed until N≥2 independent operators and a split-view drill. |
| REM-211+ | Resource metrics / Grafana | Observability polish. |
| VAL-SCALE | Expand a verified independent cohort | Waitlist size is not validator capacity. |
| ZK-CIRCUIT | Circuit/contract review only if the binding changes | Do not restore qualification. Sepolia `zk_validation` stays **false**. |

### Do not do from this repo

- Contribution Arena as protocol work (separate educational product).
- Enable ZK (`zk_validation: true`) or threshold encryption (`threshold_encryption: true`) on the signed Sepolia spec.
- Flip **CR07-ACTIVATE** / **HI11-ACTIVATE**.
- Treat the current empty-chain API as a live network, or resume fleet operations without operator gates.
- Raise the GitHub Actions spending limit.
- Commit untracked `docs/document-pdfs/`.
- `git pull origin main` (ambiguous tag named `main`). Fetch `refs/heads/main` then `merge --ff-only origin/main`.

---

## Network posture (signed spec)

File: `chain-registry/testnet/chain-spec.sepolia.json`

| Field | Value |
|-------|--------|
| `chain_id` | `creg-testnet-1` |
| `phase` | `alpha` |
| `genesis_time` | 2026-09-19T11:36:08Z |
| `genesis_hash` | `0x9d9e86adb5bbf4f6e16c04e9825ab99c209bf44c1c7560968111e859a5e594ae` |
| Spec identities | `validator-1` … `validator-4` (foundation observers, non-voting) |
| `bootnodes` | 2 listed (same GCP host) |
| `zk_validation` | `false` |
| `threshold_encryption` | `false` |
| `cross_chain` | `false` |

Public waitlist is live (Firebase). Contribution Arena is a separate product.

---

## Local verification snapshot

Recorded 2026-08-23 on Windows; details in [TEST_EVIDENCE.md](./remediation/TEST_EVIDENCE.md).

| Harness | Result |
|---------|--------|
| CLI | 49/49 |
| Node lib | 225/225 |
| `creg-node` | 227/227 |
| Node E2E | 8/8 |
| Node integration | 11/11 |
| Workspace all-target | exit 0 at `5868401` (includes sidecar 2/2) |
| `cargo fmt --all -- --check` | exit 0 |
| Foundry v1.7.1 `forge test` | 129/129 |
| GitHub Actions | Does not start (billing) |

Later PRs 41–50 added fail-closed patches after that workspace capture. Re-run local suites before treating a new slice as verified. `cargo test -p chain-registry-cli --lib` is expected to fail (no lib); use `--bin creg`.

---

## Suggested next protocol slice

If the goal is **in-repo protocol** without fake witnesses: **WITNESSED-LOG Track B** — recruit independent operators to run `creg witness run` on hosts the log operator does not control ([WITNESS_OPERATOR_ONBOARDING.md](./WITNESS_OPERATOR_ONBOARDING.md)). The daemon existing is not a witnessed network.

If the goal is a **live public testnet**: stop feature work and do **VAL-INDEPENDENCE** (recruit operators) then **TESTNET-RESUME**. GOVERNANCE-API / SHIM / freshness / SBOM do not turn `api.testnet.cregnet.dev` (empty chain, no peers) into a live network.

---

_Update this file when a protocol PR merges or hosting phase changes. [NEXT_WORK.md](./archive/NEXT_WORK.md) is archived; track new IDs here._
