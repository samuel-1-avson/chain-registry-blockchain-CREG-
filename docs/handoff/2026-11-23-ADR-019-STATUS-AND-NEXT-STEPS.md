# CREG Ecosystem Re-Architecture: Status Handoff (2026-11-23)

> **For the next prompt or agent:** this file records what was done, what was found, what is verified, and what is still open for the ADR-019 ecosystem re-architecture. Read it first. Then read:
> - [`docs/adr/ADR-019-AGENTIC-ECOSYSTEM-REARCHITECTURE.md`](../adr/ADR-019-AGENTIC-ECOSYSTEM-REARCHITECTURE.md): the plan. §9 lists the increments, §11 the open decisions and §13 the Increment 1 status.
> - [`docs/architecture/ECOSYSTEM_DEEP_DIVE_AUDIT.md`](../architecture/ECOSYSTEM_DEEP_DIVE_AUDIT.md): findings `ECA-01` to `ECA-42` with evidence and line references.

---

## 0. TL;DR

- **Done:** Phase 0 (audit plus ADR-019 plan) and **Increment 1 of 7** (client and edge "stop-bleed", no consensus wire changes). Also done: 15 Dependabot PRs merged, plus a follow-up React version fix.
- **Not done:** Increments 2–7. That is most of the original brief: the v4 producer, proof-carrying resolver, CLI v4 evidence UX and operator tooling, indexer v2, the relayer/faucet rebuild, `sled`/`ethers`/`zk-validator` retirement, the `deny.toml` burn-down, and the agentic-era explorer/hub UI modules.
- **Blocker for most remaining work:** Increment 2 needs **maintainer sign-off on ADR-019 §11 decisions #1, #3 and #4**. It also needs confirmation that the live fleet chain spec does **not** already schedule v4 activation (`hybrid_signature_activation_height` / `vote_v4_active_at`).
- **Can start now:** Increment 6 (L1 services and supply chain). It does not depend on Increment 2; its only open choice is decision #5 (relayer store = PostgreSQL, recommended).
- **Current `main`:** `69556dc`. It is pushed, and local `main` equals `origin/main`.

---

## 1. Original brief (summary) and hard rules

The original request: after ADR-018 Increments 1–7 (validator re-architecture, commit `1445d3b`), re-architect the rest of the CREG ecosystem.

- **Phase 0:** deep-dive audit plus ADR.
- **Track 1:**
  - Resolver: v4 + Hybrid PQ QC verification, a Quarantined/RevokeV2 fail-closed block, and `sled` retirement.
  - CLI: v4 evidence rendering, plus `creg doctor` / `creg ai` / `creg validator` tooling for validator-ai.toml, SecretRef, LLM connectivity and zone/sandbox probes.
  - db-sync: v4 schema and `sled` retirement.
- **Track 2:** migrate off `ethers` to alloy 0.6, harden the relayer and faucet, retire `sled`, decouple `zk-validator`, and burn down `deny.toml`.
- **Track 3:**
  - A security audit.
  - New UI modules: zone/AI committee telemetry, a `TriggerWitnessV1` inspector, a Quarantine 72h queue and a DiscoveryBounty leaderboard.

Increments are worked upstream to downstream: Track 1, then Track 2, then Track 3.

**Hard rules (from the user, keep verbatim):**

1. **OS & Shell:** Windows PowerShell 5.1. Never use `&&` to chain commands; use `; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }`.
2. **Cargo Environment:** Always set `$env:LIBCLANG_PATH = "C:\Users\samue\AppData\Local\Programs\CREG-LLVM-23.1.2\bin"; $env:CARGO_BUILD_JOBS = "2"` before running Cargo commands in `chain-registry/`.
3. **Compiler & Zero-Warning Policy:** `rustc 1.98.1`. `[workspace.lints.rust]` and `[workspace.lints.clippy]` in `chain-registry/Cargo.toml` have zero blanket allows. Every increment must pass:
   - `cargo fmt --all -- --check`
   - `cargo clippy --locked --workspace --all-targets -- -D warnings`
   - `cargo clippy --locked -p chain-registry-node --features phase-c-real-fleet --all-targets -- -D warnings`
   - `cargo deny --workspace check advisories bans sources`
4. **Frozen Test Files (Never Modify):** `chain-registry/crates/common/tests/consensus_golden_vectors.rs`; `chain-registry/crates/consensus/tests/signed_message_formats.rs`
5. **Untracked Files (Never Stage or Modify):** `chain-registry/diagnostics/`; `docs/gcp-cost-audit-2026-10-02/live-inventory.json`
6. **Git Push Disambiguation:** Because a remote tag `refs/tags/main` exists, always push with `git push origin refs/heads/main:refs/heads/main`.

**User preference:** CREG should run its own systems. Avoid third-party RPC, indexer and SaaS services where practical.

---

## 2. Environment notes (how work actually runs on this machine)

- **Shell tools:** the agent's Bash and PowerShell tools fail with `EPERM` on `AppData\Local\Temp\claude`. Run commands through the **Terminal panel**. Put anything non-trivial in a `.bat` or `.ps1` under `C:\Users\samue\.cargo-targets\phase0-logs\`, and write logs there as **ASCII**. PowerShell `*>>` produces UTF-16 logs.
- **Rust env:**
  - `CARGO_TARGET_DIR=C:\Users\samue\.cargo-targets\creg`
  - `CARGO_PROFILE_TEST_DEBUG=0`
  - plus the rule-2 variables.
- **Ready-made scripts** (all in `C:\Users\samue\.cargo-targets\phase0-logs\`):
  - `inc1-gates.bat`: the four required gates plus targeted tests. Takes about 40 min with a warm cache. The summary goes to `inc1-gates-summary.log`.
  - `inc1-js-tests.ps1`: explorer and hub-api unit tests without `npm install`. It uses Node 24 `--experimental-strip-types` and rewrites `./x.js` imports to `.ts` in a temp copy.
  - `nginx-validate\run.ps1` / `cleanup.ps1`: loads the real explorer and hub-web nginx configs into `nginx:alpine` with a stub RPC upstream, then runs `validate-edge.mjs` (183 checks).
- **JS packages:** explorer, hub-api and hub-web have **no `node_modules`**. JS builds and lint have never run in this session.
- **Gotchas:**
  - The repo `.gitignore` ignores **every `lib/` directory**; only `hub-web/src/lib` is re-included. Never put new source under `lib/`.
  - rustc denies raw bidi codepoints in string literals. Use `\u{202e}` escapes.
  - `core.autocrlf=true`, so compare text files CR-insensitively in tests.
- **CI:** GitHub Actions is billing-locked; only DCO runs, and it fails because commits carry no `Signed-off-by`. `main` has no branch protection.

---

## 3. Repository state

| Commit | What |
| --- | --- |
| `1445d3b` | Baseline: ADR-018 Increment 7 (before this work) |
| `47ea7b9` | `feat(ecosystem): ADR-019 Phase 0 audit and Increment 1 - client and edge stop-bleed`, 60 files |
| `e99711d … a03105b` | 15 Dependabot PR merge commits (#61–#75, merge-commit style) |
| `69556dc` | `fix(explorer): align react and react-is with react-dom 19.3.1` (**current `main`**, pushed) |

- **Untracked:** the two rule-5 paths, plus this handoff file (`docs/handoff/…`, not committed yet).
- **ADR-019 §13 is stale on one point:** it still says Increment 1 is "uncommitted"; correct it in the next commit.

---

## 4. Phase 0 deliverables

- **`docs/architecture/ECOSYSTEM_DEEP_DIVE_AUDIT.md`:** 42 findings (3 CRITICAL, 17 HIGH, 19 MEDIUM, 3 LOW), a component inventory, a brief-vs-code naming reconciliation, and a `deny.toml` burn-down ledger. Of the 50 advisory ignores, 15 are removable: 11 ethers-only, 2 `sled`+`ethers`, and 2 `zk-validator`.
- **`docs/adr/ADR-019-AGENTIC-ECOSYSTEM-REARCHITECTURE.md`:**
  - Chosen option: a "proof-carrying ecosystem" on one shared view model, `crates/registry-view`.
  - Invariants `ECO-INV-01..10`.
  - Seven increments.
  - Adversarial corpus `eco-adv-01..13`.
  - Eight open decisions.

**Key architectural conclusion:** the node does not yet *produce* any v4 data (`ECA-01`). The v4 functions exist only in tests:
- `aggregate_vote_v4_quorum`
- `WitnessReplayGuard`
- `resolve_quarantine_timeout`

The node also never constructs `PackageStatus::Quarantined` and always emits `RevokeV2 { witness: None }`. Every downstream v4 feature (resolver proofs, CLI evidence, indexer, explorer modules) therefore depends on Increment 2.

**The brief's names vs. the code** (from audit §2):

| Brief says | Code actually has |
| --- | --- |
| `VOTE_V4_DOMAIN` | `VOTE_V4_MESSAGE_DOMAIN` |
| `TriggerSpecV1` | `TriggerSpecWireV1` |
| `WitnessAdmissionTracker` | `WitnessReplayGuard` |
| `SemanticVoteV4` with `confidence_bp` / `finding_categories` | Neither field exists |
| `RevokeV2` with `quorum_signatures` | No such field; `trigger_witness_digest` lives on `ValidatorSignature` |
| `SecretRef` with `file:` | Only `env:` and `vault:` |
| `chain-registry/hub` | Only `hub-api` and `hub-web` exist |
| `ObservedKernelViolationV1` | No type; the field is `Vec<String>`. The real closed enum is `KernelViolationCode` (9 codes) in `validator/kernel_sandbox/schema.rs` |
| `ethers` code to migrate | `cross-chain` has **no** `ethers` code at all; the dependency is dead. Real L1 anchoring is the alloy-based `node/src/bridge.rs` |

---

## 5. Increment 1: what was implemented (commit `47ea7b9`)

No consensus wire type, signed message, block or header format changed, and the frozen test files are untouched.

### Resolver / CLI

- **ECA-07 (CRIT), quarantine hard block:**
  - New `VerdictStatus::Quarantined { reason, content_hash, expires_at_unix }` in `common/src/verdict.rs`.
  - It is blocked with no `--unverified` / `CREG_ALLOW_UNVERIFIED` override. At the deadline it becomes `quarantine_expired`, which is permanent like revoked.
  - The freshness journal gained a `Quarantined { expires_at_unix }` kind. It keeps the first-seen deadline, may only end in revoked or (before the deadline) a verified release, and becomes revoked once the deadline passes.
  - Journaled revoked or quarantined packages stay blocked when the node is unreachable. This includes PATH shims, where it was a pre-existing bypass.
  - Unrecognised node status strings fail closed.
  - Files: `resolver/{chain_client,install_gate,freshness,cache}.rs` and the CLI renderers.
- **Correction to the audit:** the node's `get_package` REST route reported quarantined records as `"pending"`. It now returns `"quarantined"` plus `quarantine_reason` and `quarantine_expires_at_unix` (`node/src/api.rs`, `openapi.rs`).
- **ECA-11:** gRPC verdict fast path deleted; `tonic`/`prost` removed from `resolver/Cargo.toml` (and `Cargo.lock`).
- **ECA-13:**
  - `resolve`/`resolve_id` (audit, status, verify, batch) use the install-grade path. Positive cache entries live at most 5 min, and `Unknown` is never cached.
  - **Bug fixed:** `creg audit --json` always reported 0 revoked packages and exited 0, because it compared a nested object with a string.
- **ECA-14:** bounded verifying downloader (`resolver/downloader.rs`).
  - Strict CID grammar, request timeout and streaming SHA-256.
  - 64 MiB cap (`CREG_MAX_DOWNLOAD_BYTES`), a `create_new` file in a private per-download `TempDir`, and losing races cancelled and cleaned up.
  - `creg install` uses the verified handle.
- **ECA-15 (partial):** journal writes `fsync` the file and no longer pre-delete it. The advisory lock and `sled` removal remain for Inc 3.
- **ECA-17:**
  - New `common::DisplayText` / `sanitize_for_terminal` (`common/src/display_text.rs`) strips C0 (except tab), DEL, C1, ESC and bidi controls, and bounds length.
  - Applied to `watch`, `info`, `install`, `audit`, `verify`, verdict output, proxy reasons and resolver block messages.
  - **Bug fixed:** `creg info` byte-sliced node strings and could panic on multi-byte values.
  - **Not yet applied:** the `creg console` TUI (`explorer_tui.rs`) and `tracing` log lines (see §8).
- **ECA-18 (Inc 1 part):** `creg watch` drops a stream after 256 KiB without a frame separator. `--ci` also fails on `package_revoked*`, `package_quarantined`, `quarantine_expired` and `witness_verified`.
- **ECA-19:**
  - New flags `--private-key-file/--private-key-ref` and `--key-file/--key-ref` (`env:VAR` or `file:PATH`; on Unix the file must be owner-only and at most 4 KiB).
  - The old argv flags are hidden and refused unless `CREG_ALLOW_ARGV_SECRETS=1`.
  - Token amounts are exact decimal-to-wei `u128` instead of `f64` (`cli/src/secret_input.rs`).

### Node / db-sync / relayer / cross-chain

- **ECA-39:** the router fails **closed**. If the state lock is contended while building the router, `/metrics` and the docs stay operator-only.
- **ECA-23:** `publisher_stats.stake_wei` is `NUMERIC(78,0)`, applied three ways: fresh DDL, an idempotent startup hot-fix, and `migrations/003_stake_wei_numeric.sql` (kept byte-equal to the hot-fix by a unit test).
- **ECA-29 (relayer):**
  - New `RELAYER_TRUSTED_PROXY_HOPS` setting; legacy `RELAYER_TRUST_PROXY=true` means 1.
  - The client is the XFF entry N counted from the **right**. Non-IP values are ignored, and IPv6 clients are bucketed by `/64`.
- **ECA-26 (Inc 1 part):** `L1RelayerService` refuses `dry_run=false` before touching its queue. Dry-run receipts are `RelayTxStatus::Simulated`, `creg relayer soak --broadcast` was removed, and the output is labelled SIMULATED.

### Web edge

- **ECA-35:**
  - Per-app `security-headers.conf` is included in **every** nginx location for the explorer and hub-web. This fixes the `add_header` inheritance bug that stripped headers from the SPA document.
  - Headers: CSP (`script-src 'self'`, `object-src 'none'`, `base-uri 'none'`, `frame-ancestors 'none'`), HSTS, `nosniff`, `X-Frame-Options: DENY`, Referrer-Policy, Permissions-Policy and COOP.
  - A docs-only CSP covers Swagger UI at `/api-docs`.
- **ECA-38:**
  - Explorer `/rpc`: POST-only, 64 KiB body cap, `limit_req` at 10 r/s per client with a burst of 20.
  - It also has an **njs JSON-RPC method allowlist** (`explorer/rpc-allowlist.js`, loaded by `explorer/nginx-main.conf`). It allows wallet reads plus the signed-raw-tx lifecycle used by `creg testnet stake-*`, and batches of at most 20.
  - Caddy Sepolia routes got a `request_body` 64 KB cap.
- **ECA-29/36 (hub-api / nginx):**
  - nginx uses realip from private-network peers, walking `X-Forwarded-For` from the right. It keeps **appending** XFF. This deliberately deviates from ADR §6.3, because the Caddy edge writes the real client into XFF.
  - hub-api's limiter (`middleware/rateLimit.ts`, `clientAddress.ts`) is LRU-bounded (`HUB_RATE_LIMIT_MAX_KEYS`) and keyed by the trusted client address. It returns 503 in production when no address can be established.
  - `/api/status/public` is served from a 7.5 s single-flight cache (`hub-api/src/status/singleFlight.ts`).
- **ECA-34 (Inc 1 part):** one mapping (`txDisplayStatus`, `packageStatusVariant` in `explorer/src/api/normalize.js`). `revoke`/`revoke_v2` show as red "revoked", with the target read from `request.package_canonical`. Quarantine is amber, and unknown types or statuses are neutral, never success.

### Adversarial corpus coverage (as colocated unit tests, not `tests/ecosystem_adversarial/`)

- `eco-adv-01`: `install_gate.rs`
- `eco-adv-02`: gRPC path deleted; `chain_client.rs`
- `eco-adv-07`: `display_text.rs`, `watch.rs`
- `eco-adv-08`: `watch.rs`
- `eco-adv-09`: `hub-api/src/middleware/rateLimit.test.ts`
- `eco-adv-12`: `explorer/src/api/normalize.test.js`
- `eco-adv-13`: `downloader.rs`
- Edge Playwright spec: `explorer/e2e/security-headers.spec.js`. Run it with `E2E_EXPECT_EDGE_HEADERS=1` against the nginx image.

---

## 6. Dependabot merges and the follow-up fix

All 15 open Dependabot PRs were merged into `main` with merge commits, one at a time. #72 and #70 conflicted and were auto-rebased by Dependabot first.

| Group | PRs |
| --- | --- |
| GitHub Actions | #61 upload-artifact 4→7, #62 checkout 4→7, #63 cosign-installer 3.7.0→4.1.3, #64 setup-java 4→6, #65 attest-build-provenance 2→4 |
| Cargo (lockfile-only) | #66 xxhash-rust, #68 clap, #69 uuid, #72 clap_complete, #74 flate2. None is a consensus-critical crate listed in `dependabot.yml`. |
| Explorer npm | #73 react-dom 19.3.1, #75 vite 8.3.8, #67 @lhci/cli 0.16.1, **#71 react-router-dom 6→7 (major)**, **#70 @walletconnect/ethereum-provider 2.11→2.26 (now bundles `@reown/appkit`)** |

**Regression found and fixed (`69556dc`):** #73 left `react`/`react-is` at 19.3.0 next to `react-dom` 19.3.1. React 19 throws error #527 ("Incompatible React versions") at startup, and `explorer/.npmrc` has `legacy-peer-deps=true`, so npm would not catch it. Fixed with a lockfile-only update to `^19.3.1`.

---

## 7. Verification evidence

| Check | Result |
| --- | --- |
| 4 required Rust gates on `47ea7b9` **and again on merged `a03105b`** | All pass |
| `cargo test -p common -p resolver -p chain-registry-cli -p db-sync -p cross-chain -p chain-registry-relayer` | All pass: CLI 109, common 100 + golden vectors 10/10, resolver 52, cross-chain 17, relayer 7, db-sync 1 |
| `consensus --test signed_message_formats` (frozen) | Pass |
| `chain-registry-node --lib api::` | 59/59, including the new quarantine-status and fail-closed-metrics tests |
| `chain-registry-node --test integration resolver_tests` | 3/3 |
| Explorer `node --test` (normalize + rpc allowlist) | 12/12, re-run after the React fix |
| hub-api limiter + single-flight under Node type stripping | 9/9 |
| nginx: real explorer and hub-web configs in `nginx:alpine` 1.31.9 with njs | `nginx -t` OK for both; **183/183** header and `/rpc` checks pass (405/403/400/413/200/429 paths, upstream never reached on denial) |

**Never run:**
- `npm ci` / `npm run build` / lint / `tsc` for explorer, hub-api and hub-web. This leaves the react-router 7 and WalletConnect 2.26 upgrades unproven.
- The Playwright runner itself.
- Caddy config load.
- The realip trust path (Docker NAT hides it).
- hub-web on its pinned `nginx:1.27-alpine`; it was tested on 1.31.9.

---

## 8. Open issues and residual gaps (not yet fixed)

**Security / correctness (higher priority):**

1. **The `/rpc` allowlist is bypassed in production.** The Caddy edge (`testnet/caddy/Caddyfile.unified`) imports `sepolia-public-rpc.caddy` on the explorer and faucet hosts and exposes `api…/sepolia`. All of these proxy directly to a **third-party** upstream (default `ethereum-sepolia-rpc.publicnode.com`) with any JSON-RPC method. Core Caddy cannot inspect bodies. Possible fixes:
   - Route Caddy `/rpc` to the explorer container.
   - Build the Inc 6 self-hosted RPC gateway. Self-hosted Sepolia nodes already exist: `testnet/sepolia-nethermind/`, `testnet/sepolia-geth/`, `testnet/gcp/*sepolia*`.
2. **Node stake overflow.** `node/src/publisher_index.rs` and `validator/src/reputation.rs` store `stake_wei` as `u64`, which saturates near 18.4 tokens. This is the same class as ECA-23 and is a candidate for Inc 2/5.
3. **Unsanitized TUI and logs.** `creg console` (`cli/src/explorer_tui.rs`) and `tracing` log lines do not yet sanitize untrusted strings. `eco-adv-07` expects TUI and log coverage.
4. **Key on `cast`'s argv.** `creg testnet stake-*` still passes the private key to Foundry `cast` via the child process's argv. The fix is an in-process signer (`crates/l1-tx`, Inc 6).
5. **Public `/metrics` passthrough.** Caddy `api…/metrics` proxies the node's `/metrics` publicly whenever the node is configured to expose it.
6. **Explorer runtime is unverified** after react-router-dom 7 and WalletConnect 2.26. AppKit fonts (`fonts.reown.com`) are blocked by the CSP `font-src`, which is cosmetic; add it or self-host if needed.
7. **hub-api `index.ts` was restructured** (status handler moved into `buildPublicStatus`) and has not been type-checked with real `hono` types.

**Deferred by design (scheduled):**

- CSP Trusted Types, removing `style-src 'unsafe-inline'`, narrowing `connect-src https: wss:`, and self-hosting Google Fonts (third-party; conflicts with the self-hosted preference). Inc 7.
- Freshness journal cross-process advisory lock, and `sled` removal from the resolver (Inc 3). `sled` removal from the relayer and db-sync is Inc 6/5.
- `creg audit` exit-code scheme 2/3/4 (quarantined/revoked/unproven) is Inc 4. Today quarantined and revoked both exit 1.
- The adversarial corpus lives as unit tests next to the code. ADR-019 named `tests/ecosystem_adversarial/`; move it or update the ADR.

**Process:**

- GitHub Actions is billing-locked, so the five Actions major-version bumps (including `actions/checkout` v7) are unexercised.
- DCO fails on every commit (no `Signed-off-by`).
- ADR-019 §13 says "uncommitted"; update it.
- This handoff file is not committed.

---

## 9. Findings status (all 42)

Legend:
- ✅ done
- ◐ partially done (the Inc 1 part shipped)
- ⬜ not started

| ID | Sev | Summary | Status | Remaining in |
| --- | :---: | --- | :---: | :---: |
| ECA-01 | CRIT | v4 artifacts never produced | ⬜ | Inc 2 |
| ECA-02 | CRIT | Single-signer `RevokeV2`, permanent client block | ⬜ | Inc 2/3 |
| ECA-03 | HIGH | Lane C LLM in consensus process; public LLM prose | ⬜ | Inc 2/7 |
| ECA-04 | HIGH | zone-ipc shared token; no replay/peer-cred | ⬜ | Inc 4 |
| ECA-05 | HIGH | No v4 read contract (OpenAPI/DTO/SSE) | ⬜ | Inc 2 |
| ECA-06 | MED | Open-ended v4 display vocabulary | ◐ (`DisplayText` exists) | Inc 2 |
| ECA-07 | CRIT | Quarantined installable via break-glass | ✅ | — |
| ECA-08 | HIGH | Proof verification compile-gated off in `creg` | ⬜ | Inc 3 |
| ECA-09 | HIGH | Ed25519-only PBFT check | ⬜ | Inc 3 |
| ECA-10 | HIGH | No current-status / non-revocation proof | ⬜ | Inc 2/3 |
| ECA-11 | HIGH | gRPC version confusion + TLS downgrade | ✅ | — |
| ECA-12 | HIGH | Freshness checks inert | ⬜ | Inc 3 |
| ECA-13 | HIGH | 24h cache drives audit/verify | ✅ | — |
| ECA-14 | MED | Unbounded downloader | ✅ | — |
| ECA-15 | MED | Cache/journal crash safety; `sled` | ◐ | Inc 3 |
| ECA-16 | HIGH | CLI links validator/zk/ml/wasm/cross-chain | ⬜ | Inc 4 |
| ECA-17 | MED | Terminal escape injection | ◐ (TUI/logs left) | Inc 1 follow-up |
| ECA-18 | MED | SSE buffer; CI blind to v4 | ◐ | Inc 4 (SSE v2 resume) |
| ECA-19 | MED | Secrets on argv; `f64` amounts | ✅ (`cast` child argv left) | Inc 6 |
| ECA-20 | MED | Signature-derived publisher key; wildcard loopback CORS | ⬜ | Inc 4 |
| ECA-21 | LOW | No zone/sandbox/AI operator diagnostics | ⬜ | Inc 4 |
| ECA-22 | HIGH | Indexer schema blind to v4 | ⬜ | Inc 5 |
| ECA-23 | HIGH | `stake_wei BIGINT` overflow | ✅ (node `u64` left) | Inc 2/5 |
| ECA-24 | MED | Schema drift, no migrations, cursor outside tx | ⬜ | Inc 5 |
| ECA-25 | MED | In-process indexer; per-block RocksDB reopen | ⬜ | Inc 5 |
| ECA-26 | HIGH | Fabricated "live" L1 relay | ◐ (refused) | Inc 6 (retire crate) |
| ECA-27 | MED | Dead `ethers` (11 deny ignores) | ⬜ | Inc 6 |
| ECA-28 | HIGH | Hot-wallet EOA nonce race | ⬜ | Inc 6 |
| ECA-29 | MED | Spoofable client IP | ✅ | — |
| ECA-30 | MED | Single RPC; receipt watcher gives up | ⬜ | Inc 6 |
| ECA-31 | MED | Faucet PoW challenge store DoS | ⬜ | Inc 6 |
| ECA-32 | MED | Relayer/faucet secret and transport hygiene | ⬜ | Inc 6 |
| ECA-33 | MED | `sled`, `zk-validator`, permissive bans | ⬜ | Inc 6 |
| ECA-34 | HIGH | Explorer shows revocation/quarantine as success | ◐ (badges fixed) | Inc 7 (generated enums, timeline) |
| ECA-35 | HIGH | No CSP/HSTS; header inheritance bug | ✅ (Trusted Types deferred) | Inc 7 |
| ECA-36 | HIGH | hub-api limiter bypass/leak/amplification | ✅ | — |
| ECA-37 | MED | SSE/WS caps, no resume, polling | ⬜ | Inc 7 |
| ECA-38 | MED | Open `/rpc` proxy | ◐ (explorer image only; Caddy edge open) | edge follow-up / Inc 6 |
| ECA-39 | MED | Metrics/docs fail-open | ✅ | — |
| ECA-40 | LOW | SIWE nonce race; session hygiene | ⬜ | Inc 7 |
| ECA-41 | LOW | `javascript:` markdown sink | ⬜ | Inc 7 |
| ECA-42 | MED | Unpaid committee bounty share; L1-only event | ⬜ | Inc 5/7 (+ contract fix) |

---

## 10. What to do next (recommended order)

### Step A: decisions needed from the maintainer (ADR-019 §11)

| # | Decision | Recommendation | Blocks |
| --- | --- | --- | --- |
| 1 | Current-status proof mechanism (ECA-10) | `BlockHeader.status_root`, a sparse Merkle tree, bundled into v4 activation | Inc 2, 3 |
| 2 | Quarantine override | None (already implemented in Inc 1) | — |
| 3 | `RevokeV2` authority post-activation (ECA-02) | Owner/controller self-revocation **or** `CommitteeQuorumCertificateV1` (≥ 2m/3+1) | Inc 2 |
| 4 | `SemanticVoteV4` extension | Add optional `confidence_bp` (display-only) and a closed `finding_categories` list before activation | Inc 2, 5, 7 |
| 5 | Relayer durable store | PostgreSQL (same pattern as the faucet) | Inc 6 |
| 6 | Operator diagnostics channel (ECA-04) | HKDF per-link zone-ipc keys + read-only `ZoneId::Operator` | Inc 4 |
| 7 | LLM prose on public surfaces (ECA-03) | Remove; quantized fields only | Inc 2, 7 |
| 8 | Pre-activation install policy | k-of-n (2-of-3) observer agreement from the signed spec | Inc 3 |

Also **confirm the live fleet chain spec does not already set `hybrid_signature_activation_height` / `vote_v4_active_at`**. If it does, the Inc 2 wire additions must move to a follow-up activation (`creg-vote-v4.1`).

### Step B: quick follow-ups from Increment 1 (small, independent)

- Close the Caddy `/rpc` and `/sepolia` gap: route to the explorer allowlist or a self-hosted gateway, and point upstreams at the self-hosted Sepolia nodes instead of publicnode.
- Sanitize the `creg console` TUI and log output (`DisplayText`).
- Run `npm ci && npm run build` (and the Playwright header spec against the built image) for the explorer. Run `tsc --noEmit` for hub-api and hub-web.
- Widen the node's `u64` stake fields, after checking whether they are part of consensus state.
- Update ADR-019 §13 ("uncommitted") and commit this handoff file.

### Step C: increments (see ADR-019 §9 for exit gates)

| Inc | Scope | Depends on |
| --- | --- | --- |
| **2** | v4 producer completion and public read contract. Items: wire the node's `validator_pipeline.rs` and `namespace_consensus.rs` to `aggregate_vote_v4_quorum` and `WitnessReplayGuard`; Quarantined transactions with 72h expiry; `CommitteeQuorumCertificateV1` + `verify_committee_qc`; `RevokeV2.quorum_certificate`; `BlockHeader.status_root`; `QuarantineV1`/`QuarantineResolutionV1`; promote `KernelViolationCode` into `common`; new `crates/registry-view` DTOs; v4 REST routes + SSE v2 + OpenAPI; move Lane C (LLM) out of `creg-consensusd` into `creg-agent-runner`. All new fields `#[serde(default, skip_serializing_if…)]`, golden vectors byte-identical. | Decisions 1/3/4/7 |
| **3** | Proof-carrying resolver. Items: signed-spec authority always (remove the phase-C compile gate); per-signature `verify_pbft_phase_signature_at_height` (hybrid PQ); SMT status proof + tip freshness; journal v2 (content-hash pin, height monotonicity, certified stickiness, advisory lock); JSON proof cache replacing `sled`. | Inc 2 |
| **4** | CLI v4 evidence UX and operator tooling. Items: evidence panels; `creg evidence verify-witness`; TUI tabs; `crates/ai-config` with `file:` SecretRef; `creg ai check` / `creg ai probe`; `creg validator doctor` over the zone-ipc operator link (`ZoneRequest::Capabilities` / `ProbeProvider`); CLI crate diet (drop validator/zk/ml/wasm/cross-chain); login key-binding redesign (ECA-20). | Inc 2 (v4 parts) |
| **5** | Indexer v2. Items: `crates/indexer` (RocksDB secondary); `sqlx::migrate!` schema v2 (vote signatures, semantic votes, witnesses, revocations, quarantines, validator events); single-tx apply + continuity checks; L1 `DiscoveryBountyAwarded` ingest via `common::l1_rpc`; `/v1/index/*` API; remove the in-process worker from consensusd. | Inc 2 |
| **6** | L1 services and supply chain. Items: retire `crates/cross-chain` (move merkle helpers to `common` test-support); `crates/l1-tx` SignerActor + durable EOA nonce ledger over `common::l1_rpc`; relayer → PostgreSQL (+ `sled` export/import); faucet stateless HMAC PoW, loopback admin, PG TLS + pool; `deny.toml` −15 ignores, bans on `sled`/`ethers`, `wildcards = "deny"`, licenses allowlist; unlink `zk-validator`. | **Independent; can start now** (decision #5) |
| **7** | Explorer and hub agentic-era UI. Items: status/revocation timeline; zone/AI committee telemetry; `TriggerWitnessV1` inspector; 72h quarantine queue; DiscoveryBounty leaderboard (committee 75% labelled "reserved" until the `ValidatorRewards.sol` fix, ECA-42); `<UntrustedText>`; Trusted Types; SSE-first data layer; SIWE/session fixes; markdown sink removal. | Inc 2 + 5 |

**Suggested immediate path:**
1. Get decisions 1/3/4/5/7 signed off and check the live chain spec.
2. Then either start **Inc 2** (critical path) or, while waiting, run **Step B** and **Inc 6**.

---

## 11. Key file pointers

- Plan and status: `docs/adr/ADR-019-AGENTIC-ECOSYSTEM-REARCHITECTURE.md` (§3 Inc 2 design, §4 Track 1, §5 Track 2, §6 Track 3, §9 increments, §11 decisions, §13 status)
- Audit: `docs/architecture/ECOSYSTEM_DEEP_DIVE_AUDIT.md`
- v4 wire types: `chain-registry/crates/common/src/vote_v4.rs`, `common/src/block.rs` (`RevokeV2` ~L262), `common/src/package.rs` (`PackageStatus`, `ValidatorSignature`)
- v4 consensus logic (unused by node): `chain-registry/crates/consensus/src/vote_v4.rs`
- Node producer side: `chain-registry/crates/node/src/validator_pipeline.rs`, `namespace_consensus.rs` (`validate_revoke_v2`), `api.rs`, `events.rs`, `proof.rs`, `intelligence/`
- Resolver: `chain-registry/crates/resolver/src/{install_gate,chain_client,freshness,cache,downloader,light_client}.rs`
- Terminal sanitizer: `chain-registry/crates/common/src/display_text.rs`
- Edge: `chain-registry/explorer/{nginx.conf,nginx-main.conf,security-headers*.conf,rpc-allowlist.js}`, `chain-registry/hub-web/{nginx.conf,security-headers.conf}`, `chain-registry/hub-api/src/middleware/`, `chain-registry/testnet/caddy/`
