# Remediation Backlog

Track security and remediation work from [archive/SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md](./archive/SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md).

| ID | Status | Phase | Notes |
|----|--------|-------|-------|
| DOC-101 | done | 1 | Docs index + root README links |
| DOC-102 | done | 1 | API cookbook |
| DOC-103 | done | 1 | Security ops runbook |
| DOC-104 | done | 1 | Backlog sync |
| REM-101 | done | 1 | Explorer `relayer.js` → `/sponsor`, `/status/:id`, `/policy`, `/quote` |
| REM-102 | done | 1 | `migrations/001_db_sync_bootstrap.sql`, `002_testnet_extras.sql` |
| SEC-104 | done | 1 | Rate limits on `/v1/publisher/packages`, `/v1/validator/consensus/vote` |
| SEC-201 | done | 1 | ZKVerifier — 6/6 tests pass (`forge test --match-contract ZKVerifier`) |
| SEC-102 | done | 1 | `validate_production_security()` fail-fast at node boot |
| SEC-106 | done | 1 | `creg doctor` PBFT + production safety checks |
| REM-201 | done | 2 | Governance HTTP 501 + explorer hidden unless `VITE_GOVERNANCE_ENABLED=true` |
| REM-103 | done | 2 | JSON cursor sidecar + idempotency/reorg tests; atomic save |
| REM-210 | done | 2 | [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) |
| SEC-203 | done | 2 | `creg chain-spec validate` — genesis hash + optional `.sig` Ed25519 verify |
| SEC-101 | done | 2 | Hot-key runbook rotation + `.env.example` placeholders |
| SEC-101b | done | 2 | Startup WARN with fingerprint (bridge, faucet, relayer) when not testnet |
| REM-103b | done | 2 | Chunked `eth_getLogs` (10k blocks, `CREG_ETH_LOG_CHUNK_BLOCKS`) + cursor advance on empty deltas; Sepolia restart synced in ~10s vs 9m cold walk |
| SEC-105 | done | 2+ | `creg keygen` warning + `creg stake` rejects Ed25519 key file; [WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md) |
| REM-211 | done | 2+ | Live scrape verified (`creg_node_sepolia` UP); Grafana import optional |
| REM-203 | done | 2+ | Merged [PR #6](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/6) — workspace alloy 0.6; `/metrics` validator_set_sync |
| SEC-303c | done | 3 | D4: cross-chain **Planned** / disabled; spec `cross_chain: false`; see [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) |
| SEC-302 | deferred | 3 | CrossChainRegistry ISSUE-005/006 — execute only if D4 reversed |
| SEC-306a | done | 3 | D5: PrivateRegistry **Planned** — README + contracts/README |
| SEC-304 | done | 3 | `CREG_SHIELDED_PUBLISH_ENABLED` default false; node + CLI |
| SEC-305 | done | 3 | Shared `shielded_wire` format (CLI ↔ node); admission skips YARA on ciphertext; `admission_accepts_shielded_when_enabled` + common round-trip tests |
| SEC-301a | done | 3 | [adr/ADR-KMS-HOT-KEYS.md](./adr/ADR-KMS-HOT-KEYS.md) — testnet env/Vault; mainnet requires SEC-301b |
| SEC-301b | done | 3 | `chain-registry-secrets` crate — env + Vault KV2; wired to node/faucet/relayer |
| GAPS-2026-06 | in-progress | Audit + Hardening | Iteration 6 complete (batch + relays). Doc hub: `docs/WHAT_TO_DO_NEXT.md`. Remaining: Phase 0 evidence pack, Phase 2 bridge/ZK, testnet resume. See `docs/WEAKNESSES_GAPS_REMEDIATION_PLAN.md` §6 |
| DOC-302 | done | 3 | 2026-06-26 — Doc hygiene: WHAT_TO_DO_NEXT, archive obsolete docs, gcp/README, fix broken links |
| DOC-303 | open | Audit | Committee metadata vs analysis sampling alignment — `is_mandatory` / `should_perform_full_analysis` not yet tied to admission committee; see WHITEPAPER §1 alpha note + WORKFLOW §9.11 |
| CR-04/HI-11 | in-progress | Testnet readiness | Authenticated validator-set transitions, checkpoint-descended history, peer continuity checks, mandatory certified-committee gates, focused 8/8 node E2E, and the signed four-candidate fresh-genesis spec are green. Independent custody/bootnode evidence, the full-workspace aggregate, and live multi-operator rehearsal remain. |
| CR-07 | in-progress | Testnet readiness | Namespace authority API, lifecycle replay protection, archive quotas/GC, pinned npm/PyPI sidecars, and fail-closed trust-candidate acquisition are implemented. Production trust approval, positive fixtures, and independent review remain. |
| CR-07-SBX | in-progress | Hardening | Sidecar execution now uses bounded streaming digests, private scratch cleanup, Unix process groups, Windows Job Objects, protected scratch DACL inheritance, and a whole-invocation deadline. OS-level deployment rehearsal remains. |
| DOC-304 | done | Evidence | Workflow/whitepaper Markdown was corrected for ceiling-derived quorum and HI-11 semantics; PDFs, public/Functions mirrors, visual QA, and manifest hashes were refreshed on 2026-08-01. |

**Phase 2 ship (2026-05-28):** Sepolia Option A on `main`. **Phase 3 epics:** largely complete in code; open work tracked in [NEXT_WORK.md](./NEXT_WORK.md) and `WEAKNESSES_GAPS_REMEDIATION_PLAN.md`.

_Update status when PRs merge._
