# CREG testnet — external phase scope

**Status:** Phase open (limited)  
**Network:** Sepolia L1 + `creg-testnet-1` (signed chain spec)  
**Effective:** 2026-05-30  
**Review:** 2026-06-30 or when [NET-301](./NEXT_WORK.md) (multi-validator Sepolia) ships

This page defines what external participants can expect from the **current** testnet. It is not a mainnet commitment.

---

## What this testnet is

A **coordinated Sepolia deployment** of Chain Registry: publishers submit signed packages; validators (when deployed) analyze and finalize; observers sync L1 validator-set state and expose APIs.

**Canonical operator docs:** [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) · [SEPOLIA_SECOND_OPERATOR_CHECKLIST.md](./SEPOLIA_SECOND_OPERATOR_CHECKLIST.md)

---

## Node roles

| Mode | `CREG_IS_VALIDATOR` | What it does |
|------|---------------------|--------------|
| **Observer** (default in reuse scripts) | `false` | Syncs Sepolia staking/registry; serves API; admits publishes into **pending**; does **not** run local PBFT finalization |
| **Validator** | `true` + `CREG_VALIDATOR_KEY` + stake | Runs analysis, votes, and can drive packages to **verified** on the local chain |

Until **NET-301** (multi-validator Sepolia), the project runs a **single-observer testnet** for external smoke: one known `creg-node` per environment, not a decentralized validator fleet.

---

## Package statuses (what “verified” means)

| Status | Meaning on API / `creg status` |
|--------|--------------------------------|
| **pending** | Admitted to the node’s in-memory pending pool; consensus not complete on this node |
| **UNVERIFIED** | CLI label for pending (install allowed with warnings per product policy) |
| **verified** | Record on the node’s RocksDB chain after validator pipeline + finalization |
| **revoked** | Rejected or revoked on chain |
| **UNKNOWN** | Not on chain and not in pending (wrong node, restart, wrong URL, or cache) |

**Verified** means this **node’s** chain store has accepted the package after validator workflow—not “every node on the internet agrees” until multi-node testnet is proven.

---

## Known limits (read before integrating)

1. **Pending pool is in-memory** — Restarting `creg-node` drops pending submissions; re-publish if needed.
2. **Observer nodes keep pending visible** — As of `validator_pipeline` observer fix, observers no longer delete pending entries after ~1s (rebuild required).
3. **No cross-chain** — `feature_flags.cross_chain: false` in spec ([D4](./PHASE3_KICKOFF.md)); bridge UI/receipts deferred.
4. **Governance API disabled** — HTTP 501 by design (REM-201); explorer governance gated.
5. **Shielded publish** — Off unless `CREG_SHIELDED_PUBLISH_ENABLED=true` on client and node (experimental, SEC-304/305).
6. **CLI / REST footguns** — Always pass `--node-url` (or `CREG_NODE_URL`) for local testnet; URL-encode canonicals in REST paths (`@` and `/` break unencoded routes).
7. **Bootnodes / public IPFS in spec** — Example hostnames; production testnet fleet not operated by this repo alone.
8. **Single-observer period** — Multi-validator Sepolia (**NET-301**) targeted for review by **2026-06-30**; until then, do not assume PBFT quorum across independent operators.

---

## What is in scope for external participants

- Run an **observer** against published `chain-spec.sepolia.json` + signature
- **Publish** (staked publisher, IPFS, Ed25519 key) and read **pending** status
- Integrate **public REST** (`/v1/public/*`) and health/metrics
- Report issues against pinned commit on `main`

## Out of scope (this phase)

- Mainnet or economic guarantees
- Cross-chain verification
- On-chain private registries (Planned / D5)
- Production KMS for all hot keys (in progress; see [ADR-KMS-HOT-KEYS.md](./adr/ADR-KMS-HOT-KEYS.md))
- Formal security audit completion (scheduled; [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md))

---

## Phase-open checklist (maintainers)

| Step | Done |
|------|------|
| Observer pending-pool fix on `main` | Yes |
| E2E-301 publish smoke documented and verified | Yes |
| OPS-201 sign-off | [SEPOLIA_SECOND_OPERATOR_CHECKLIST.md](./SEPOLIA_SECOND_OPERATOR_CHECKLIST.md) |
| This scope page published | Yes |
| NET-301 or dated single-observer decision | Single-observer until **2026-06-30** (see above) |

---

## Contact / issues

Use the repository issue tracker listed in the chain spec `support.issues` field. Security: `support.security` in spec.
