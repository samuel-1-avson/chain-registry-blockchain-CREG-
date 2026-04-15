# Chain Registry — UI, CLI & Web Explorer Deep Dive
> Version: 0.1.0 | Analysis Date: 2026-04-15

## Table of Contents
1. [Executive Summary](#1-executive-summary)
2. [Testnet Readiness Scorecard](#2-testnet-readiness-scorecard)
3. [CLI System](#3-cli-system)
   - 3.1 Feature Inventory
   - 3.2 Bugs & Stubs
   - 3.3 Missing for Testnet
   - 3.4 Security Gaps
   - 3.5 UX Issues
4. [TUI Explorer](#4-tui-explorer)
   - 4.1 Feature Inventory
   - 4.2 Bugs & Stubs
   - 4.3 Missing for Testnet
   - 4.4 Security Gaps
   - 4.5 UX Issues
5. [Web Explorer](#5-web-explorer)
   - 5.1 Feature Inventory
   - 5.2 Bugs & Stubs
   - 5.3 Missing for Testnet
   - 5.4 Security Gaps
   - 5.5 UX Issues
6. [API Layer (Shared Foundation)](#6-api-layer-shared-foundation)
7. [Issue Registry](#7-issue-registry)
   - 7.1 Critical Severity
   - 7.2 High Severity
   - 7.3 Medium Severity
   - 7.4 Low Severity
8. [Improvement Roadmap](#8-improvement-roadmap)
   - 8.1 Before Public Testnet (Week 1)
   - 8.2 Testnet Hardening (Week 2–3)
   - 8.3 Quality-of-Life (Week 4)
9. [Appendix — File Map](#9-appendix--file-map)

---

## 1. Executive Summary

The Chain Registry project has three user-facing subsystems for interacting with the blockchain:

| Subsystem | Files | Lines | Framework |
|-----------|-------|-------|-----------|
| **CLI** (`creg`) | 30+ Rust files | ~12,000 | Clap v4, Ratatui, reqwest |
| **TUI Explorer** | `explorer_tui.rs` | 2,090 | Ratatui 0.27 |
| **Web Explorer** | `App.jsx` + assets | 2,763 | React 19, Viem, Tailwind |

**Overall verdict**: The CLI and TUI are feature-complete and testnet-ready with known gaps. The Web Explorer is near-ready but has critical input validation holes and one unauthenticated admin endpoint in the shared API. **One critical security issue (unauthenticated package revocation) must be fixed before any public testnet launch.**

---

## 2. Testnet Readiness Scorecard

| Subsystem | Score | Blocking Issues | Notes |
|-----------|-------|-----------------|-------|
| **CLI** | **82 / 100** | Retry logic missing (C-19) | Feature-complete; safety culture (`#![deny(clippy::unwrap_used)]`) is excellent |
| **TUI Explorer** | **75 / 100** | Offline detection absent | Beautiful real-time UI; no consensus voting view |
| **Web Explorer** | **68 / 100** | Input validation missing; dev-key in prod | Contract config required before deploy |
| **API (shared)** | **55 / 100** | **CRITICAL: unauthenticated revoke** | Single unauthenticated write endpoint blocks entire testnet |

**Combined readiness**: ~72 / 100 — 1–2 weeks of focused work from public testnet.

---

## 3. CLI System

### 3.1 Feature Inventory

The CLI (`creg`) is the primary user-facing tool. It covers the complete publisher and validator workflow.

**Package Management**
- `install`, `status`, `verify`, `audit`, `search`, `info`, `graph`, `diff`
- Lockfile management (`lockfile add/remove/check/export`)
- Batch operations (`batch install/verify/audit`)
- SBOM export (CycloneDX, SPDX)

**Publishing**
- `publish` — single-signer with PGP, Ed25519, ZK proof
- `multisig publish` — M-of-N offline signing with `.creg-multisig.json` session files
- `publish --shielded` — threshold-encrypted submission
- Offline signing for air-gapped workflows

**Key Management**
- `keygen` — Ed25519 key generation with BIP39 mnemonic
- `keygen --recover` — mnemonic restore
- Key rotation, social recovery (Shamir guardian shares)

**Validator / Staking**
- `stake publisher`, `stake validator` — Ethereum ERC20 approve + stake via alloy
- `testnet drip/stake/status/reset` — local testnet lifecycle management

**Developer Tools**
- `doctor` — 8-point system health check (node, IPFS, GPG, etc.)
- `shims install/uninstall` — transparent npm/pip/cargo/gem/mvn interceptors
- Shell completions (bash, zsh, fish, PowerShell)
- `advanced` — raw API calls, ZK witness generation, WASM inspect

**TUI Integration**
- `creg explore` — launches Ratatui dashboard (see §4)

### 3.2 Bugs & Stubs

| ID | File | Line | Issue | Severity |
|----|------|------|-------|----------|
| CLI-B01 | `install.rs` | 4–6 | TODO C-19: no retry logic on transient network failures | High |
| CLI-B02 | `install.rs` | 6 | TODO C-22: org-level policy enforcement stubbed out | Medium |
| CLI-B03 | `install.rs` | 6 | TODO C-23: config_file values not threaded through all commands | Medium |
| CLI-B04 | `publish.rs` | 273, 298 | `.expect()` on hardcoded ProgressBar template strings | Low |
| CLI-B05 | `publish.rs` | ~420 | Offline signing emits ZK score `85/100` placeholder; validators re-score at submission | Medium |
| CLI-B06 | `main.rs` | 765 | `.expect("cannot determine current directory")` — no user-actionable message | Low |
| CLI-B07 | `multisig.rs` | ~180 | Session files stored on disk with no HMAC integrity check | Medium |
| CLI-B08 | `advanced.rs` | 84–90 | Sandbox unavailability treated as `safe=false` and silently continues in dev mode | Medium |
| CLI-B09 | `shims/` | — | Windows PATH manipulation untested; Unix-specific assumptions in shim installer | High |

### 3.3 Missing for Testnet Readiness

**Priority 1 — Must-have**
1. **Retry with exponential backoff** (CLI-B01) — `install`, `publish`, `verify` fail immediately on transient 5xx/network errors. A flaky testnet node causes false "package unavailable" reports.
2. **Windows shim support** (CLI-B09) — shim installer uses Unix PATH semantics. Windows validators and publishers get a broken installation experience.
3. **Config file full integration** (CLI-B03) — `~/.creg/config.toml` values are parsed but not consistently applied to all subcommands.

**Priority 2 — Should-have**
4. **Org-level policy evaluation** (CLI-B02) — Enterprise users need `CREG_POLICY_FILE` support in `install` to enforce department-wide allow/deny rules.
5. **Multisig session integrity** (CLI-B07) — Sign HMAC over `(canonical, threshold, signers, unsigned_tx)` so co-signers cannot modify session files between rounds.
6. **PGP fallback** — `publish` requires GPG installed; CI systems often don't have it. Add `--no-pgp` flag with explicit warning.

**Priority 3 — Nice-to-have**
7. **BIP39 passphrase support** in `keygen --recover`
8. **Batch error aggregation** — Partial failures in `batch install` lose individual error context
9. **Success deep-link** — After `publish`, print `creg://pkg/<canonical>` or explorer URL

### 3.4 Security Gaps

| ID | Severity | File | Description |
|----|----------|------|-------------|
| CLI-S01 | High | `install.rs:124` | `--unverified` flag leaves no audit trail in lockfile receipt; silent continue on receipt error |
| CLI-S02 | Medium | `publish.rs:~280` | Offline signing placeholder ZK score `85/100` is not removed before submission; validator rewrites it but lockfile copy retains fake score |
| CLI-S03 | Medium | `multisig.rs:~180` | `.creg-multisig.json` session files have no integrity check; a malicious co-signer can alter threshold or canonical before signing |
| CLI-S04 | Low | All | `RUST_LOG=debug` prints full request/response bodies; ensure no private key material appears in request bodies |

### 3.5 UX Issues

1. **No progress for ZK proof generation** — Groth16 takes 5–10 s on first run; terminal appears frozen.
2. **`doctor` output not machine-readable** — Cannot use in CI pipelines without parsing human text.
3. **Shim output too verbose** — `shims install` prints 6+ colored lines per manager; should collapse to 1.
4. **No `--dry-run` for publish** — Cannot preview what will be submitted to the network.
5. **`verify` output format inconsistent** — Some errors are JSON, some are plain text, depending on `--json` flag propagation.

---

## 4. TUI Explorer

### 4.1 Feature Inventory

The TUI (`creg explore`) is a 2,090-line Ratatui 0.27 real-time dashboard with 10 distinct views.

**Views available**

| View | Key | Contents |
|------|-----|----------|
| Overview | `1` | Chain stats, validator count, package count, mempool depth |
| Blocks | `2` | Scrollable block list with height, hash, tx count, timestamp |
| Block Detail | `Enter` on block | Transaction list, proposer, prev hash, merkle root |
| Validators | `3` | Stake, reputation, status (online/offline/self), P2P address |
| Validator Detail | `Enter` on validator | Full identity, pubkey, recent votes |
| Packages | `4` | Name, version, publisher, content hash, IPFS CID |
| Package Detail | `Enter` on package | Full manifest, findings summary, ZK proof status |
| Network | `5` | P2P peer list, latency, bridge sync, ETH finalization |
| Mempool | `6` | Pending packages with submission timestamp |
| Events | `7` | Live SSE event stream (block produced, vote cast, etc.) |
| Operator | `8` | Node ID, config path, uptime, finalized tx queue depth |

**Navigation**
- Vim keys: `hjkl`, `gg`/`G`, `/` (search), `Enter` (drilldown), `Esc` (back)
- `?` toggles help overlay
- 100 ms tick rate; 3 s data refresh interval

### 4.2 Bugs & Stubs

| ID | File | Line | Issue | Severity |
|----|------|------|-------|----------|
| TUI-B01 | `explorer_tui.rs` | ~500 | No explicit state machine validation; malformed SSE JSON could panic on unwrap | Medium |
| TUI-B02 | `explorer_tui.rs` | ~1200 | Unverified packages render name as `"unknown"` without any warning indicator | Low |
| TUI-B03 | `explorer.rs` | 19–22 | Hardcoded `/ui/` path prefix; SPA routing conflict if mounted at root | Low |
| TUI-B04 | `explorer_tui.rs` | ~900 | Cache eviction is FIFO with fixed limits (MAX_EVENTS=200, MAX_BLOCKS=100) — no warning when full | Low |
| TUI-B05 | `explorer_tui.rs` | ~300 | SSE disconnect logged but UI shows no "offline" banner or reconnect countdown | High |

### 4.3 Missing for Testnet Readiness

**Priority 1 — Must-have**
1. **Offline/disconnect detection** (TUI-B05) — When the node SSE stream drops, the TUI freezes silently. Must show "⚠ DISCONNECTED — retrying in Xs" banner and auto-reconnect.
2. **Consensus voting view** — No view shows live PBFT round progress (PRE-PREPARE → PREPARE → COMMIT quorum counts). Essential for debugging validator issues during testnet.
3. **Validator sort/filter** — Large validator sets are unnavigable without sort-by-stake and reputation filter.

**Priority 2 — Should-have**
4. **Bridge status detail** — Bridge sync shown as raw string; needs formatted ETH block confirmations, latency, and error state.
5. **Copy-to-clipboard** — Cannot copy hashes or addresses; requires manual terminal selection.
6. **Pagination on all tables** — Large block/validator/package lists need `Page Down`/`Page Up` with count indicator.

**Priority 3 — Nice-to-have**
7. Export view state to file (`e` key → write current view JSON)
8. Dark/light theme toggle
9. Small terminal mode (< 80 cols)

### 4.4 Security Gaps

| ID | Severity | Description |
|----|----------|-------------|
| TUI-S01 | Medium | Operator view exposes validator node IDs and P2P multiaddresses — if TUI is shared over `tmux` or forwarded, this leaks topology |
| TUI-S02 | Low | No schema validation before parsing SSE events; crafted gossip could trigger unwrap panics (see TUI-B01) |

### 4.5 UX Issues

1. **No loading spinners** — 3 s initial data fetch shows blank tables; looks broken.
2. **Help is hidden** — `?` shortcut undiscoverable; first-time users get stuck.
3. **Bridge status string is opaque** — Raw "synced" / "0x1234..." provides no actionable info.
4. **Package detail has no IPFS gateway link** — Users must manually construct `https://ipfs.io/ipfs/<CID>`.
5. **Validator detail shows raw pubkey hex** — No truncation or copy prompt; 64-char hex is unreadable.

---

## 5. Web Explorer

### 5.1 Feature Inventory

A React 19 SPA (`explorer/src/App.jsx`, 2,763 lines) using Viem for Ethereum wallet integration.

**Pages / Sections**

| View | Contents |
|------|----------|
| Blocks | Block list with height, hash, tx count, timestamp; click → detail |
| Validators | Stake, reputation, status; click → detail with pubkey |
| Packages | Name, version, publisher, findings badge; search/filter |
| Wallet | Account balance, CREG token balance, connected address |
| Staking | Approve + stake as publisher or validator with amount input |
| Publish | Manual form: name, version, IPFS CID, content hash, pubkey, signature |
| Network Profiles | Switch between Anvil (dev), Sepolia, Hoodi with configurable RPC |
| Operator Panel | Node health, bridge status, validator registrations |

**Wallet support**: MetaMask, WalletConnect, raw private key (dev mode only)

**Real-time**: SSE event stream drives live updates for blocks, validators, packages

### 5.2 Bugs & Stubs

| ID | File | Line | Issue | Severity |
|----|------|------|-------|----------|
| WEB-B01 | `App.jsx` | 18 | `PRIVATE_KEY_WALLET_ENABLED` allows private key input if `VITE_DEV_MODE='true'` — no build-time guard prevents this in a production bundle | High |
| WEB-B02 | `App.jsx` | 160, 192, 213 | `console.error()` calls for fetch failures — no user-visible error state, no retry | Medium |
| WEB-B03 | `App.jsx` | 608–634 | Silent fallback on runtime config fetch; explorer may display wrong network mode | Medium |
| WEB-B04 | `App.jsx` | 887 | `alert()` box to warn about private key mode — should be a disabled button with tooltip | Low |
| WEB-B05 | `App.jsx` | 1365, 1446 | Swallowed errors on package list/profile fetch — no retry, no user feedback | Medium |
| WEB-B06 | `App.jsx` | ~1800 | Publish form has zero input validation before POST — empty name, fake IPFS CID, garbage signature all accepted | Critical |
| WEB-B07 | `App.jsx` | ~650 | SSE reconnect not implemented; on disconnect the live updates stop silently | High |

### 5.3 Missing for Testnet Readiness

**Priority 1 — Must-have**
1. **Publish form input validation** (WEB-B06) — Validate IPFS CID format (`Qm...` or `bafy...`), content hash (64-char hex), pubkey (64-char hex), signature (128-char hex) before POST.
2. **Multisig UI** — No way to upload `.creg-multisig.json` or track M-of-N signature collection. CLI-exclusive today, but testnet publishers need web parity.
3. **Contract address config** — `VITE_CREG_TOKEN`, `VITE_STAKING_ADDR`, `VITE_REGISTRY_ADDR` may be empty at deploy time. Add a startup check that displays a clear "configure your contracts" message.
4. **SSE reconnect** (WEB-B07) — Implement exponential backoff reconnect so live view doesn't silently freeze.

**Priority 2 — Should-have**
5. **Transaction history** — After staking, no way to check the transaction hash, retry, or see confirmations.
6. **Package revocation UI** — Validators/operators need a web panel to revoke a package with evidence (requires fixing API-C01 first).
7. **Reputation legend** — The 0–100 score is shown but never explained; users don't know what "reputation 72" means.
8. **Pagination on package/block lists** — No lazy loading; 10,000+ entries will hang the browser.

**Priority 3 — Nice-to-have**
9. Dark mode toggle
10. `creg install <name>` copy button next to every package
11. QR code / shareable URL per package

### 5.4 Security Gaps

| ID | Severity | Description |
|----|----------|-------------|
| WEB-S01 | High | `PRIVATE_KEY_WALLET_ENABLED` — if `VITE_DEV_MODE=true` is set at build time, private key entry is shown in prod; key stored as React state with no encryption |
| WEB-S02 | Medium | WalletConnect session tokens stored unencrypted in `localStorage`; an XSS on the explorer domain exposes connected wallet |
| WEB-S03 | Medium | Unauthenticated publish form — anyone can POST arbitrary package data; the API validates the signature but the form provides no guidance on what a valid signature is |
| WEB-S04 | Medium | Contract addresses come from `VITE_*` env vars baked into the JS bundle; build-system compromise could point staking to attacker-controlled contract |
| WEB-S05 | Low | `console.error()` logs API endpoint URLs and error bodies — attackers can inspect DevTools to discover internal routes |

### 5.5 UX Issues

1. **No success animation on stake** — After clicking "Stake", button disappears with no visual confirmation; users re-click, causing double transactions.
2. **Network profile switch is invisible** — Profile selector in top-right is easy to miss; users may unknowingly be on wrong network.
3. **IPFS CID input accepts anything** — No format hint, no live validation, silent failure at submission.
4. **Error messages are HTTP codes** — "HTTP 403: Insufficient stake" should say "You need to stake at least 0.01 CREG first. Go to Staking tab."
5. **Validator registration flow has no wizard** — Users must know to navigate Operator → Register → fill 3 fields; no guided flow.
6. **Copy tooltip races** — Clicking copy twice within 2 s leaves stale tooltip visible.
7. **Tables are unsortable** — Cannot sort blocks by height, validators by stake/reputation, packages by name.

---

## 6. API Layer (Shared Foundation)

The three subsystems all depend on the node REST API (`crates/node/src/api.rs`). Issues here affect all clients.

### 6.1 Implemented Endpoints

| Method | Path | Auth | Notes |
|--------|------|------|-------|
| GET | `/v1/packages` | None | List all packages |
| POST | `/v1/packages` | Sig | Submit package (signature verified) |
| GET | `/v1/packages/:canonical` | None | Get one package |
| **POST** | **`/v1/packages/:canonical/revoke`** | **NONE** | **⚠ UNAUTHENTICATED** |
| GET | `/v1/packages/:canonical/proof` | None | SPV proof |
| GET | `/v1/blocks/:height` | None | Block by height |
| GET | `/v1/blocks/hash/:hash` | None | Block by hash |
| POST | `/v1/blocks/announce` | None | P2P block announcement |
| GET | `/v1/validators` | None | Validator set |
| GET | `/v1/nodes` | None | Node metadata |
| GET | `/v1/p2p/status` | None | P2P peer list |
| GET | `/v1/bridge/status` | None | ETH bridge state |
| SSE | `/v1/events` | None | Server-sent events stream |

### 6.2 API Issues

See `ISSUE-API-C01` in §7 for the critical revoke gap. Additional issues:

- `/v1/blocks/announce` (POST) has no authentication — any node can inject fake block announcements
- `/v1/p2p/status` leaks full peer list and multiaddresses to any caller — topology disclosure
- No rate limiting on any endpoint (global rate limiter exists in node but not enforced per-endpoint)
- Pagination missing on `/v1/packages` (list) and `/v1/blocks` — no `?page=` or `?limit=` params

---

## 7. Issue Registry

### ISSUE-API-C01: Unauthenticated Package Revocation Endpoint

- **Severity**: Critical
- **File**: `crates/node/src/api.rs:711`
- **Description**: `POST /v1/packages/:canonical/revoke` accepts any caller with only `{ "reason": "string" }`. There is no signature verification, no publisher/governance check, and no rate limiting. `revoked_by` is hardcoded to the string `"api-request"` — the actual caller identity is never recorded.
- **Impact**: Any unauthenticated user knowing a package's canonical name can permanently revoke it from the network. On a public testnet, all verified packages are vulnerable to denial-of-service revocation.
- **Attack vector**: `curl -X POST http://<node>:8080/v1/packages/npm:express@4.18.2/revoke -d '{"reason":"malware"}'`
- **Recommended Fix**:
  ```rust
  #[derive(Deserialize)]
  struct RevokeReq {
      reason: String,
      revoker_pubkey: String,   // hex Ed25519 pubkey of the caller
      signature: String,        // Ed25519 sig over "{canonical}:revoke:{reason}"
  }
  // Before queuing the tx:
  verify_revocation_sig(&canonical, &req.reason, &req.revoker_pubkey, &req.signature)?;
  // revoker must be a known validator or the original publisher
  require_validator_or_publisher(&state, &req.revoker_pubkey)?;
  ```

---

### ISSUE-CLI-H01: No Retry Logic on Network Failures

- **Severity**: High
- **File**: `crates/cli/src/install.rs:4–6` (TODO C-19)
- **Description**: `install`, `publish`, and `verify` commands make HTTP requests with no retry logic. A single 5xx response or TCP timeout causes an immediate hard failure.
- **Impact**: On a testnet with occasional node restarts, publishers get false "package unavailable" errors. Users must manually re-run commands.
- **Recommended Fix**: Use the existing `retry.rs` module (which implements exponential backoff) uniformly across all HTTP-calling commands. Default: 3 retries, 500 ms base, 2× backoff, jitter.

---

### ISSUE-CLI-H02: Windows Shim Installer Untested

- **Severity**: High
- **File**: `crates/cli/src/shims/`
- **Description**: The shim installer modifies shell PATH using Unix-specific approaches (`.bashrc`, `.zshrc`, symlinks). Windows users get a broken install; no `.bat` or PowerShell shims are generated.
- **Impact**: Windows validators and publishers cannot use transparent package interception. The `creg shims install` command on Windows silently does nothing useful.
- **Recommended Fix**: Detect OS at runtime; on Windows generate `.cmd` batch file shims in `%USERPROFILE%\.creg\bin\` and add that directory to the user-level `PATH` registry key.

---

### ISSUE-WEB-H01: Publish Form Has No Input Validation

- **Severity**: High
- **File**: `explorer/src/App.jsx:~1800`
- **Description**: The web publish form POSTs to `/v1/packages` with whatever the user typed. No format checks on IPFS CID, content hash, pubkey, or signature fields. Empty or garbage submissions reach the node, causing cryptic server errors.
- **Impact**: Poor developer experience; security researchers can fuzz the API endpoint via the UI. Confusing error messages cause testnet participants to give up.
- **Recommended Fix**: Add client-side validators:
  ```js
  const CID_RE = /^(Qm[1-9A-HJ-NP-Za-km-z]{44}|b[a-z2-7]{58})$/;
  const HEX64_RE = /^[0-9a-fA-F]{64}$/;
  const HEX128_RE = /^[0-9a-fA-F]{128}$/;
  if (!CID_RE.test(ipfsCid)) setError("Invalid IPFS CID format");
  ```

---

### ISSUE-WEB-H02: SSE Stream Does Not Reconnect

- **Severity**: High
- **File**: `explorer/src/App.jsx:~650`
- **Description**: The `EventSource` for live updates is opened once; on disconnect (`onerror`) the UI stops updating silently. There is no reconnect loop or visual indicator.
- **Impact**: After any node restart or network hiccup, the web explorer appears live but is actually frozen. Testnet participants get stale block/validator data without knowing it.
- **Recommended Fix**:
  ```js
  function connectSSE() {
    const es = new EventSource('/v1/events');
    es.onerror = () => { es.close(); setTimeout(connectSSE, 3000); };
    // … attach handlers
  }
  connectSSE();
  ```
  Show a "⚠ Reconnecting…" banner while disconnected.

---

### ISSUE-TUI-H01: No Disconnect Detection or Reconnect

- **Severity**: High
- **File**: `crates/cli/src/explorer_tui.rs:~300`
- **Description**: SSE disconnect is logged at `tracing::warn!` level but the TUI keeps displaying stale data indefinitely. No visual indicator, no auto-reconnect.
- **Impact**: Testnet operators watching the TUI during a node restart believe the chain is healthy when it is actually stopped.
- **Recommended Fix**: Track `last_event_at: Instant`. In the render loop, if `last_event_at.elapsed() > STALE_THRESHOLD (10s)`, overlay a red "⚠ NODE DISCONNECTED" banner. On reconnect, resume normally.

---

### ISSUE-CLI-M01: Multisig Session File Has No Integrity Check

- **Severity**: Medium
- **File**: `crates/cli/src/multisig.rs:~180`
- **Description**: `.creg-multisig.json` session files are JSON stored on disk and passed between co-signers. A malicious co-signer can edit the threshold, canonical name, or previously collected signatures before adding their own.
- **Impact**: A 2-of-3 multisig could be downgraded to 1-of-3 by a malicious co-signer. Package canonical name could be changed after other signers have approved.
- **Recommended Fix**: HMAC the session file fields at creation time using the initiator's private key. Each subsequent signer verifies the HMAC before adding their signature.

---

### ISSUE-WEB-M01: Private Key Wallet Exposed in Production Builds

- **Severity**: Medium
- **File**: `explorer/src/App.jsx:18`
- **Description**: `PRIVATE_KEY_WALLET_ENABLED = import.meta.env.DEV || import.meta.env.VITE_DEV_MODE === 'true'`. If `VITE_DEV_MODE=true` is set at build time for a production deployment, the private key input field is shown. The key is stored as unencrypted React state.
- **Impact**: Production users could be prompted to paste their private key into a browser form. Key may be accessible via browser DevTools memory inspection.
- **Recommended Fix**: Remove `VITE_DEV_MODE` check entirely. Private key mode should only be available in `import.meta.env.DEV` (Vite's compile-time dev flag, never true in production builds).

---

### ISSUE-API-M01: Block Announcement Endpoint Unauthenticated

- **Severity**: Medium
- **File**: `crates/node/src/api.rs:200`
- **Description**: `POST /v1/blocks/announce` accepts block announcements from any caller without verifying the proposer's VRF proof or validator membership.
- **Impact**: Any attacker can inject fake block announcements, potentially triggering unnecessary sync operations or confusing the block producer.
- **Recommended Fix**: Require a proposer signature over the block hash before processing the announcement.

---

### ISSUE-API-M02: P2P Status Leaks Network Topology

- **Severity**: Medium
- **File**: `crates/node/src/api.rs`
- **Description**: `GET /v1/p2p/status` returns the full list of peer multiaddresses to any unauthenticated caller.
- **Impact**: Attacker can map the entire validator P2P network topology and target specific nodes for DDoS.
- **Recommended Fix**: Restrict to authenticated operator requests only, or return only aggregate counts (`{"peer_count": 12}`) for public callers.

---

### ISSUE-WEB-M02: No Transaction History for Staking

- **Severity**: Medium
- **File**: `explorer/src/App.jsx:~1200`
- **Description**: After staking, the transaction hash is shown in a toast but not persisted anywhere. If the toast is dismissed, there is no way to retrieve the tx hash or check confirmation status.
- **Impact**: Testnet participants cannot debug failed staking transactions. Support burden increases.
- **Recommended Fix**: Store recent transactions in `localStorage` under `creg_tx_history`. Show them in a "Recent Transactions" panel in the Wallet tab.

---

### ISSUE-CLI-L01: ZK Proof Generation Has No Progress Indicator

- **Severity**: Low
- **File**: `crates/cli/src/publish.rs`
- **Description**: Groth16 ZK proof generation can take 5–10 s on first run with no progress feedback. The terminal appears frozen.
- **Impact**: Users believe the command has hung and Ctrl-C, losing progress.
- **Recommended Fix**: Show a spinner (`indicatif::ProgressBar` with spinner style) during proof generation, then replace with "✓ ZK proof generated (6.2s)".

---

### ISSUE-TUI-L01: Consensus Voting View Absent

- **Severity**: Low (Medium for testnet debugging)
- **File**: `crates/cli/src/explorer_tui.rs`
- **Description**: No TUI view shows live PBFT consensus round state (current phase, prepare vote count, commit vote count, quorum threshold).
- **Impact**: During testnet, diagnosing why a package is stuck in the mempool requires reading raw logs. Operators have no real-time consensus visibility.
- **Recommended Fix**: Add a `Consensus` view (`9` key) showing active rounds: `block_hash | phase | prepare N/Q | commit N/Q | view# | age`.

---

### ISSUE-WEB-L01: Validator Reputation Score Has No Legend

- **Severity**: Low
- **File**: `explorer/src/App.jsx`
- **Description**: Reputation scores (0–100) are displayed next to validators but never explained. There is no tooltip, legend, or color coding.
- **Impact**: Testnet participants cannot interpret whether a score of 72 is good or bad, or what causes it to change.
- **Recommended Fix**: Add a color band (0–40 red, 41–70 yellow, 71–100 green) and a tooltip: "Reputation reflects accurate, timely validation history. Score decreases on missed votes or invalid findings."

---

### ISSUE-API-L01: No Pagination on List Endpoints

- **Severity**: Low
- **File**: `crates/node/src/api.rs`
- **Description**: `GET /v1/packages` and `GET /v1/blocks` return all records. With 10,000+ packages, this causes slow responses and browser-side hangs in the web explorer.
- **Recommended Fix**: Add `?limit=50&offset=0` query params. Default limit: 100. Return `X-Total-Count` header.

---

## 8. Improvement Roadmap

### 8.1 Before Public Testnet (Week 1)

These items **block testnet launch**:

- [ ] **ISSUE-API-C01** — Fix unauthenticated revoke endpoint: add `revoker_pubkey` + `signature` fields, verify caller is validator or publisher
- [ ] **ISSUE-WEB-H01** — Add client-side input validation to publish form (CID, hex fields)
- [ ] **ISSUE-WEB-H02** — Implement SSE reconnect in web explorer with "Reconnecting…" banner
- [ ] **ISSUE-TUI-H01** — Add disconnect detection + banner in TUI explorer
- [ ] **ISSUE-WEB-M01** — Remove `VITE_DEV_MODE` private-key path from production builds
- [ ] **ISSUE-API-M01** — Authenticate block announcement endpoint with proposer signature
- [ ] **ISSUE-CLI-H01** — Wire `retry.rs` into `install`, `publish`, `verify` commands

### 8.2 Testnet Hardening (Week 2–3)

- [ ] **ISSUE-CLI-H02** — Windows shim installer (.cmd + PowerShell + registry PATH)
- [ ] **ISSUE-CLI-M01** — HMAC integrity check on multisig session files
- [ ] **ISSUE-API-M02** — Restrict `/v1/p2p/status` to authenticated callers or aggregate-only
- [ ] **ISSUE-WEB-M02** — Persist staking tx history to localStorage, add Recent Transactions panel
- [ ] **ISSUE-TUI-L01** — Add Consensus view (`9` key) with PBFT round state
- [ ] **ISSUE-WEB-L01** — Validator reputation color bands + tooltip
- [ ] **ISSUE-API-L01** — Pagination on `/v1/packages` and `/v1/blocks`
- [ ] **ISSUE-CLI-B03** — Full config file integration across all subcommands
- [ ] **ISSUE-CLI-B02** — Org-level policy enforcement in `install`

### 8.3 Quality-of-Life (Week 4)

- [ ] **ISSUE-CLI-L01** — ZK proof generation spinner
- [ ] TUI copy-to-clipboard for hashes/addresses
- [ ] Web dark mode toggle
- [ ] TUI export view state to file
- [ ] Web `creg install <name>` copy button per package
- [ ] CLI `publish --dry-run`
- [ ] TUI Packages tab: IPFS gateway hyperlink
- [ ] CLI `doctor --json` output for CI pipelines

---

## 9. Appendix — File Map

| File | Lines | Role |
|------|-------|------|
| `crates/cli/src/main.rs` | 1,166 | CLI entrypoint, all command definitions |
| `crates/cli/src/install.rs` | 274 | Package install, verify, status |
| `crates/cli/src/publish.rs` | 675 | Package publish, PGP signing, ZK proof |
| `crates/cli/src/multisig.rs` | ~400 | M-of-N offline signing workflow |
| `crates/cli/src/advanced.rs` | ~350 | Raw API, ZK witness, WASM inspect |
| `crates/cli/src/stake.rs` | ~200 | ETH staking via alloy |
| `crates/cli/src/keygen.rs` | ~250 | Key generation, BIP39, Shamir recovery |
| `crates/cli/src/explorer_tui.rs` | 2,090 | Ratatui TUI dashboard |
| `crates/cli/src/retry.rs` | ~80 | Exponential backoff (NOT YET WIRED) |
| `crates/cli/src/doctor.rs` | ~200 | System health check |
| `crates/cli/src/shims/` | ~300 | npm/pip/cargo/gem/mvn interceptors |
| `crates/node/src/api.rs` | ~800 | REST API (Axum) |
| `crates/node/src/explorer.rs` | 52 | Static file server for web SPA |
| `explorer/src/App.jsx` | 2,763 | React 19 web explorer SPA |
| `explorer/src/main.jsx` | ~10 | Vite entrypoint |

---

*Report generated 2026-04-15. All file paths relative to `chain-registry/chain-registry/`.*
