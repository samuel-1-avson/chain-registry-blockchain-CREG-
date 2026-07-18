# Public Testnet Quickstart

> **Fleet status:** The public GCP fleet (`api.testnet.cregnet.dev`) may be **offline** while hosting is paused. Check [TESTNET_STATUS.md](./TESTNET_STATUS.md) before integrating. Local nodes and operator-run endpoints still work.

One-page guide for **publishers**, **developers**, and **validators** joining the Chain Registry Sepolia testnet (`creg-testnet-1`).

**Read first:** [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) — defines what “verified” means today and known limits.

**Public API (when fleet is up):** `https://api.testnet.cregnet.dev` — set `CREG_NODE_URL` to this (or pass `--node-url`).

---

## Before you start

| Requirement | Notes |
|-------------|--------|
| **Node URL** | Set `CREG_NODE_URL=https://api.testnet.cregnet.dev` for the public fleet. Default `http://localhost:8080` is for local dev only. |
| **Sepolia RPC** | For staking: `SEPOLIA_RPC_URL` or `CREG_ETH_RPC` |
| **Staking Tooling** | Required for contract interactions |
| **IPFS** | For **public testnet** publishes, set `CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev` so fleet validators can fetch your tarball during admission. Local Kubo (`ipfs daemon` on `127.0.0.1:5001`) is fine for local dev only. |
| **Two key types** | **Ed25519** (`creg keygen`) signs packages; **secp256k1 EOA** stakes tCREG on L1. |

**Contract addresses (Sepolia defaults):**

| Contract | Address |
|----------|---------|
| Staking | `0xf28C63C4Aafd27025E535Ab9ab7B4daC18C96Bc2` |
| CREG Token | `0x97c21d46B3eac604e92E907D54aA92eEc0Af550b` |
| Registry | `0x3aCfF05d00AC199412a94326eD8aA874aaA3596c` |

Minimum stakes (from chain spec): **1 tCREG** publisher, **100 tCREG** validator.

---

## Publisher — publish a package

### 1. Install the CLI

Download and install the latest `creg` command-line utility:

```bash
curl -sSf https://install.cregnet.dev/creg.sh | sh
```

### 2. Generate an Ed25519 publish key

```bash
creg keygen publisher --out ~/.creg/publisher.key
```

### 3. Fund and stake (Sepolia tCREG)

Use a **separate Ethereum wallet** (not the Ed25519 key file):

```bash
export SEPOLIA_RPC_URL=https://sepolia.infura.io/v3/YOUR_KEY
export CREG_STAKING_ADDR=0xf28C63C4Aafd27025E535Ab9ab7B4daC18C96Bc2

# Stake command (approve + stakeAsPublisher)
creg stake --amount 1 --role publisher \
  --key ~/.creg/publisher-eoa.key \
  --rpc-url "$SEPOLIA_RPC_URL"
```

Acquire tCREG via the public faucet operator.

### 4. Pin to fleet IPFS and publish

For the **public testnet**, point the CLI at the fleet IPFS gateway so admission can fetch your tarball (localhost-only CIDs cause 502/timeouts on `api.testnet.cregnet.dev`):

```bash
export CREG_NODE_URL=https://api.testnet.cregnet.dev
export CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev

# Local dev only: run `ipfs daemon` and omit CREG_IPFS_URL (defaults to http://127.0.0.1:5001)

creg publish ./my-package-1.0.0.tgz \
  --key-file ~/.creg/publisher.key \
  --publisher-address 0xYourPublisherAddress \
  --ecosystem npm
```

You can override IPFS per command: `creg publish ... --ipfs-url https://ipfs.testnet.cregnet.dev`

Run `creg doctor` before publishing — it warns when `CREG_NODE_URL` is remote but `CREG_IPFS_URL` is still localhost.

### 5. Track status

```bash
creg status npm:my-package@1.0.0 --node-url "$CREG_NODE_URL"
```

**Expected today:** `pending` on observer nodes; `verified` only after validator finalization.

---

## Developer — install and verify

### Install a verified package

```bash
export CREG_NODE_URL=https://api.testnet.cregnet.dev

creg install npm:lodash@4.17.21
# Add --unverified to allow pending/unknown (not recommended for production)
```

### Verify with light-client proof

```bash
creg verify npm:lodash@4.17.21 --node-url "$CREG_NODE_URL"
```

### Optional: package-manager shims

```bash
creg setup-shims
# Shims warn on unverified installs but may fall through to npm/pip on chain errors.
# Prefer `creg install` for verified IPFS-backed installs.
```

### Browse the registry

You can browse registry statistics and verified package lists via the public registry dashboard:

https://explorer.testnet.cregnet.dev

---

## Validator — join the network

At public-testnet scale CREG uses a **hybrid mandatory + voluntary analysis committee** model (see `CREG_SYSTEM_WORKFLOW.md` §9.11 for details):

- A small mandatory core (target 100 for large sets) is selected at admission time (VRF-seeded + stake-weighted + per-`eth_address` cap).
- Only mandatory members are required to run the full (expensive) pipeline.
- Any validator can voluntarily run the pipeline; their votes are recorded and improve liveness/reputation.
- Committee membership for any package is queryable at `/v1/packages/:canonical/committee`.

**External validators (high level):**

1. Stake **≥ 100 tCREG** and apply on L1:

```bash
creg stake --amount 100 --role validator \
  --key ~/.creg/validator-eoa.key \
  --rpc-url "$SEPOLIA_RPC_URL"
```

2. Generate Ed25519 validator key; register identity on the node API (`POST /v1/validators/register`).
3. Wait for consensus admission (`approveByConsensus` from an active validator).
4. Run `creg-node` with `CREG_IS_VALIDATOR=true`, `CREG_VALIDATOR_KEY`, signed chain spec URL.

Real (non-dev) sandbox + pinned scanner profile are required for your votes to count toward public quorum. Voluntary analysis is always allowed and encouraged.

---

## Common failures

| Symptom | Fix |
|---------|-----|
| `Publisher has no on-chain stake` | Run `creg stake --amount 1 --role publisher` with tCREG + EOA key |
| `Insufficient stake` / 403 on publish | Same; confirm `stakedBalance > 0` on Staking contract |
| `expected value at line 1 column 1` on publish | UTF-8 BOM in `package.json` inside tarball |
| `validator_set_sync=degraded` | Use archive-capable Sepolia RPC; set `epoch_block_height` in chain spec |
| `UnknownValidator` on node | Complete L1 registration requirements |
| `Consensus timeout` | Align scanner environment profiles across validators |
| Wrong node / UNKNOWN status | Set `CREG_NODE_URL`; pending is lost on node restart |
| Publish 502 / admission timeout | Set `CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev` when using public API; confirm CID with `curl -I "https://ipfs.testnet.cregnet.dev/ipfs/<cid>"` |

---

## Alpha limitations

### Certified committee rollout

The default public-alpha path retains compatibility behavior until operators publish a verified validator-set checkpoint. New or rebuilt nodes must receive the exact signed certificate through `CREG_WEAK_SUBJECTIVITY_CHECKPOINT`; do not copy a certificate solely from gossip or one HTTP peer. Rebuild an observer canary first, compare `GET /v1/validator/set-transitions` with two independently controlled peers, and verify that missing, conflicting, long-range, and fork-anchored histories are rejected. `CREG_REQUIRE_CERTIFIED_COMMITTEES=true` is a fail-closed rollout control and must be enabled only after the staged operator checkpoint rehearsal and recovery drill documented in `testnet/OPERATOR.md`.

- **SEC-401** external security audit not yet complete — treat as public alpha, not production.
- **L1 bridge anchoring** requires operator bridge configuration.
- **Cross-chain** and **shielded publish** remain disabled in chain spec.
- **Governance HTTP API** returns 501 by design.
