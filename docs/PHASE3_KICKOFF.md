# Phase 3 kickoff — Security & multi-chain

**Date:** 2026-05-28  
**Parent:** [SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md](./SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md) Section 5  
**Phase 2 baseline:** [PHASE2_CLOSEOUT.md](./PHASE2_CLOSEOUT.md) (Sepolia Option A shipped on `main`)

## Phase 2 closure status

| Gate (plan Section 4) | Status |
|------------------------|--------|
| Sepolia documented & repeatable | Done (`REM-210`, reuse script) |
| Validator set survives restart | Done (`REM-103` / `REM-103b`) |
| Governance honest (wire or disable) | Done — **D3** disable (`REM-201`; `REM-202` deferred) |
| Hot-key rotation exercised once | **Open** — runbook exists (`SEC-101`); schedule drill |
| `REM-203` alloy unified | Done — merged [PR #6](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/6) |
| Second engineer runbook | **Open** — [SEPOLIA_SECOND_OPERATOR_CHECKLIST.md](./SEPOLIA_SECOND_OPERATOR_CHECKLIST.md) |

Optional P2 (not blocking Phase 3): **REM-204** (api.rs ACL split), **REM-212** (soak CI).

---

## Decision D4 — Cross-chain (recorded)

**Choice:** **SEC-303c** (disable / “Planned” path) — **default per implementation plan**

| Path | When to use |
|------|-------------|
| **SEC-303c** (selected) | Sepolia testnet does **not** require L2 verification receipts yet |
| **SEC-302a/b** | Product commits to multi-chain on Sepolia; fix ISSUE-005/006 first |

**What this means now:**

- `CrossChainRegistry.sol` stays **Planned** in README and contract table.
- Chain spec keeps `feature_flags.cross_chain: false` (Sepolia reuse).
- `cross-chain` crate documents config-only status (see crate module docs).
- No explorer cross-chain surface today; nothing to gate beyond docs.
- **SEC-302** remains in backlog as **deferred** until product reverses D4.

To switch to **SEC-302**, update this section, set acceptance criteria from plan Epic 3.1, and run `forge test --match-contract CrossChainRegistry`.

---

## Decision D5 — PrivateRegistry (recorded)

**Choice:** **SEC-306a** — mark **Planned** (default per implementation plan)

- No `PrivateRegistry.sol` in the tree; README + [contracts/README.md](../chain-registry/contracts/README.md) updated.
- **SEC-306b** (implement contract + tests) deferred until enterprise commitment.

---

## Epic 3.3 — Shielded publish (SEC-304 / SEC-305)

| Item | Status |
|------|--------|
| **SEC-304** | Done — `CREG_SHIELDED_PUBLISH_ENABLED` defaults **false**; node admission + CLI gate |
| **SEC-305** | Partial — `--shield` hidden from `creg publish --help`; E2E round-trip still open |

---

## Phase 3 execution order (from plan)

1. **Epic 3.1** — Cross-chain: hold **SEC-302** (deferred) or execute after D4 reversal  
2. **Epic 3.2** — **SEC-306a** PrivateRegistry → **Planned** (**D5** done)  
3. **Epic 3.3** — **SEC-304** done; **SEC-305** E2E remaining  
4. **Epic 3.4** — **SEC-301a/b** done — `chain-registry-secrets` (env + Vault); production rejects `CREG_SECRETS_BACKEND=env` when not testnet  
5. **Epic 3.5** — **SEC-401** audit scope, **SEC-402** partition test, **SEC-307** rate-limit ADR  
6. **Epic 3.6** — **REM-205** explorer refactor (P3)

Track status in [REMEDIATION_BACKLOG.md](./REMEDIATION_BACKLOG.md).

## Phase 3 exit criteria (reminder)

- Cross-chain fixed **or** explicitly disabled in product (D4 → **disabled** for now)  
- PrivateRegistry status accurate (**D5** → Planned)  
- Shielded publish gated + tested or demoted (**SEC-304** done; **SEC-305** E2E open)  
- KMS ADR approved; testnet implementation started  
- Audit scheduled with scope  
