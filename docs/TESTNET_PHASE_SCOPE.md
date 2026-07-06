# CREG testnet — external phase scope

> **Fleet status:** The public GCP testnet stack is **paused (cost)** as of 2026-06-26. Waitlist and Contribution Arena remain live. See [TESTNET_STATUS.md](./TESTNET_STATUS.md) for the current matrix and resume steps.

**Status:** Public alpha — coordinated Sepolia testbed (fleet may be offline)  
**Network:** Sepolia L1 + `creg-testnet-1`  
**Effective:** 2026-05-30  
**Review:** 2026-07-30 or when security audit completes

This page defines what external participants can expect from the **current** testnet. It is not a mainnet commitment.

---

## What this testnet is

A **coordinated Sepolia deployment** of Chain Registry: publishers submit signed packages; validators analyze and finalize; observers sync L1 validator-set state and expose APIs.

---

## Node roles

| Mode | Validator Mode | What it does |
|------|----------------|--------------|
| **Observer** | Disabled | Syncs Sepolia staking/registry; serves API; admits publishes into **pending**; does **not** run local consensus finalization |
| **Validator** | Enabled | Runs analysis, votes, and can drive packages to **verified** on the local chain |

**Analysis at scale (hybrid committees):** For large validator sets, the expensive security pipeline is performed by a small mandatory committee (target 100, VRF-seeded, stake-weighted, capped) selected at admission time. Any validator may contribute voluntarily. The mandatory core is required for the primary package quorum; the full set participates in PBFT block finality.

**Consensus Quorum:** The network runs consensus validators with consensus quorum; publish reaches **`verified`** with validator agreement. **Sandbox Isolation:** Production runs real sandbox execution environment. Local dev runs in restricted local profile.

---

## Package statuses (what “verified” means)

| Status | Meaning on API / `creg status` |
|--------|--------------------------------|
| **pending** | Admitted to the node’s pending pool (`pending_pool.json` on disk); consensus not complete on this node |
| **UNVERIFIED** | CLI label for pending (install allowed with warnings per product policy) |
| **verified** | Record on the node’s database chain after validator pipeline + finalization |
| **revoked** | Rejected or revoked on chain |
| **UNKNOWN** | Not on chain and not in pending (wrong node, restart, wrong URL, or cache) |

**Verified** means this **node’s** chain store has accepted the package after validator workflow (mandatory committee evidence + PBFT block finality for large sets). On the public fleet, use `CREG_NODE_URL=https://api.testnet.cregnet.dev` (or the operator URL you trust).

Committee membership for any package (mandatory vs voluntary participants) is available at `/v1/packages/:canonical/committee`.

---

## Known limits (read before integrating)

1. **Pending pool persistence** — Submissions persist to `pending_pool.json` under the node data dir. Standalone or misconfigured nodes may still lose in-flight work on crash or manual deletion.
2. **Observer nodes keep pending visible** — Observers maintain pending entries until consensus resolution completes.
3. **No cross-chain** — Cross-chain bridge features and UI integration are deferred in this release.
4. **Governance API disabled** — Administrative governance actions are restricted in the public explorer.
5. **Shielded publish** — Shielded publication features are disabled by default.
6. **CLI / REST syntax** — Always specify the node endpoint URL when making queries. Ensure package IDs are URL-encoded in REST paths.
7. **Public storage gateways** — Package storage gateways are operated across multiple independent nodes.
8. **Dev sandbox profile** — Local dev may set `CREG_DEV_SANDBOX=true` to skip behavioural sandbox (SB012 advisory only). **Public Sepolia validators must not** — fleet health requires `CREG_DEV_SANDBOX=false` and real isolation (nsjail/gVisor/Docker). See [OPS_HARDENING_RUNBOOK.md](./OPS_HARDENING_RUNBOOK.md) MAL-001.
9. **Public URLs** — Live endpoints are subject to updates as the network matures.
10. **Committee selection** — At current scale the mandatory analysis core is small and deterministic (VRF + stake + cap). Voluntary contributions are always possible and recorded.

---

## What is in scope for external participants

- Run an **observer** against published chain spec details
- **Publish** (staked publisher, IPFS, Ed25519 key) and read **pending** status
- Integrate **public REST** (`/v1/public/*`) and health/metrics
- Report issues against pinned releases

## Out of scope (this phase)

- Mainnet or economic guarantees
- Cross-chain verification
- On-chain private registries
- Production Key Management Systems for hot keys
- Formal security audit completion

---

## Phase-open status

- **Observer node functionality verified:** Yes
- **E2E publish smoke test passed:** Yes
- **Consensus finalization active:** Yes
- **Public hosting and certificates configured:** Yes
- **Waitlist application deployed:** Yes

---

## Contact / issues

Use the support issue tracker listed in the active chain spec.
