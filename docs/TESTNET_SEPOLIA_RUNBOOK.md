# Sepolia testnet runbook

Operational guide for deploying Chain Registry L1 contracts on **Ethereum Sepolia** (chain ID `11155111`), publishing the signed **chain spec**, and pointing validators at the network.

**Canonical artifacts**

| Artifact | Path |
|----------|------|
| Chain spec | `chain-registry/testnet/chain-spec.sepolia.json` |
| Detached signature | `chain-registry/testnet/chain-spec.sepolia.json.sig` |
| Deployment manifest | `chain-registry/contracts/deployments/sepolia-latest.json` |
| Deploy script | `chain-registry/contracts/script/DeploySepolia.s.sol` |
| CI workflow | `chain-registry/.github/workflows/sepolia-deploy.yml` |

**CREG network identity (from spec):** `chain_id` = `creg-testnet-1`, L1 = Sepolia, `cross_chain` feature flag = `false` (Phase 3 decision still applies).

---

## Prerequisites

| Requirement | Notes |
|-------------|--------|
| **Foundry** | `forge`, `cast` on PATH (or use GitHub Actions workflow) |
| **Rust toolchain** | For `compute_genesis_hash`, `sign_chain_spec`, `verify_chain_spec` examples |
| **Sepolia ETH** | Fund deployer; typical full deploy needs non-trivial gas |
| **RPC URL** | Infura, Alchemy, QuickNode, or public RPC (rate limits apply) |
| **Secrets** | Never commit `.env.sepolia`; use CI secrets or a vault |

**Deployer key rules**

- Use a **dedicated** Sepolia deployer key (not Anvil default `0xac09…ff80`).
- Scripts refuse the Anvil key explicitly.
- Prefer **separate** keys for deployer, bridge (`CREG_BRIDGE_KEY`), and validator (`CREG_VALIDATOR_KEY`).

---

## Environment template

Copy and edit:

```powershell
copy chain-registry\testnet\.env.sepolia.example chain-registry\testnet\.env.sepolia
```

Minimum for deploy:

```env
SEPOLIA_RPC_URL=https://sepolia.infura.io/v3/YOUR_KEY
DEPLOYER_KEY=0x...
ETHERSCAN_API_KEY=          # optional verify
GOVERNANCE_THRESHOLD=2      # optional; DeploySepolia default via envOr is 2
CREG_BRIDGE_KEY=0x...       # optional; defaults to deployer in CI workflow
```

After deploy, populate node operators from the same example file (`CREG_*_ADDR`, `CREG_ETH_RPC`, `CREG_CHAIN_SPEC_URL`, etc.) — see [Node configuration](#node-configuration).

---

## Path A — Local deploy (PowerShell)

### 1. Deploy contracts

```powershell
cd f:\project\chain-registry\chain-registry
.\testnet\deploy-sepolia.ps1
```

This runs `DeploySepolia` with `--broadcast --chain-id 11155111` and writes `contracts/deployments/sepolia-latest.json`.

**Contracts deployed (production ZKVerifier, not DevZKVerifier):** Governance, Staking, Reputation, VRF, Registry, Appeal, ZKVerifier, CregToken, ValidatorRewards, PinningRewards.

### 2. Finalize chain spec

```powershell
.\testnet\finalize-sepolia-spec.ps1
```

This script:

1. Patches `testnet/chain-spec.sepolia.json` from `sepolia-latest.json`
2. Runs `compute_genesis_hash` and updates `genesis_hash`
3. Signs the spec and writes `chain-spec.sepolia.json.sig`
4. Verifies the signature

**Signing key:** Use your org’s spec signing key via environment or CI — do not rely on any hardcoded dev key in local scripts for production publishes. For CI, configure `SPEC_SIGNING_KEY` as a GitHub secret and remove local hardcoding before production use.

### 3. Publish spec (operators)

**Option 1 — Local spec server (dev)**

```powershell
copy testnet\chain-spec.sepolia.json testnet\spec-server\chain-spec.sepolia.json
copy testnet\chain-spec.sepolia.json.sig testnet\spec-server\chain-spec.sepolia.json.sig
cd testnet\spec-server
docker compose up -d
curl http://localhost:8888/chain-spec.json
```

**Option 2 — GitHub Pages** (CI input `publish_spec: true` on workflow dispatch).

**Option 3 — HTTPS CDN** in front of static `chain-spec.json` + `chain-spec.json.sig`.

Update `signing.detached_signature_url` in the spec to match the public URL of the `.sig` file.

---

## Path B — GitHub Actions deploy

1. GitHub → **Actions** → **Sepolia Deploy** → **Run workflow**
2. Inputs:
   - `deployer_key` — Sepolia-funded key (`0x…`)
   - `sepolia_rpc` — RPC URL
   - `etherscan_key` + `verify` — optional Etherscan verification
   - `publish_spec` — optional GitHub Pages publish
   - `governance_threshold` — multisig threshold (default `1` in workflow UI; script default is `2` if unset in env)

The job deploys, patches `chain-spec.sepolia.json`, computes genesis hash, signs, commits artifacts, and optionally publishes Pages.

**Review the workflow summary** for the contract address table and genesis hash before operators roll nodes.

---

## Path C — Bash (Linux / macOS / WSL)

```bash
cd chain-registry
export SEPOLIA_RPC_URL=...
export DEPLOYER_KEY=0x...
./testnet/deploy-sepolia.sh
# Then run finalize steps manually or use finalize-sepolia-spec.ps1 via pwsh
```

---

## Node configuration

Point each validator/full node at Sepolia L1 and the signed spec.

| Variable | Example / source |
|----------|------------------|
| `CREG_CHAIN_SPEC_URL` | `https://your-spec-host/chain-spec.json` or `file:///…/chain-spec.sepolia.json` |
| `CREG_CHAIN_SPEC_OFFLINE` | `false` (or `true` with cached spec on disk) |
| `CREG_SPEC_SIGNING_PUBKEY` | Must match `signing.signing_key_pubkey_hex` in spec |
| `CREG_GENESIS_HASH` | Must match `genesis_hash` in spec (node enforces if set) |
| `CREG_CHAIN_ID` | `creg-testnet-1` |
| `CREG_EXPECTED_L1_CHAIN_ID` | `11155111` |
| `CREG_ETH_RPC` | Same Sepolia RPC operators trust |
| `CREG_REGISTRY_ADDR` | From spec `contracts.registry` |
| `CREG_STAKING_ADDR` | From spec `contracts.staking` |
| `CREG_GOVERNANCE_ADDR` | From spec `contracts.governance` |
| `CREG_TOKEN_ADDR` | From spec `contracts.creg_token` |
| `CREG_TESTNET` | `true` (allows dev bypass env vars if needed) |
| `CREG_DEV_SANDBOX` | `false` for validators running real sandbox |
| `CREG_IS_VALIDATOR` | `true` if validating |
| `CREG_VALIDATOR_KEY` | Ed25519 hex (64 chars), from `creg keygen` |
| `CREG_BRIDGE_KEY` | Separate secp256k1 key if bridge enabled |

**Quick local smoke against Sepolia spec (no Docker cluster):**

```powershell
.\testnet\start-local-node.ps1 -Validator -ValidatorKey <ed25519_hex>
```

Then: `curl http://localhost:8080/v1/health` and `creg doctor --testnet`.

**Multi-validator host:** see `testnet/run-3node-host.ps1` (sets `CREG_CHAIN_SPEC_URL` per node).

---

## Post-deploy verification

```powershell
# L1 contracts exist
cast code <STAKING_ADDR> --rpc-url $env:SEPOLIA_RPC_URL

# Spec verifies
cargo run --example verify_chain_spec --package common -- `
  testnet/chain-spec.sepolia.json (Get-Content testnet/chain-spec.sepolia.json.sig)

# Genesis hash matches node expectation
cargo run --example compute_genesis_hash --package common -- testnet/chain-spec.sepolia.json

# Node doctor (with .env filled)
creg doctor --testnet
```

| Check | Pass criteria |
|-------|----------------|
| Manifest | `sepolia-latest.json` addresses match spec `contracts.*` |
| L1 chain ID | Node refuses start if RPC reports ≠ `11155111` when `CREG_EXPECTED_L1_CHAIN_ID` set |
| Spec signature | `verify_chain_spec` exits 0 |
| Validator sync | `GET /v1/chain/stats` shows `validator_set_sync` progressing (see [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md)) |

---

## Rollback and redeploy

Sepolia deployments are **immutable** at the contract level. “Rollback” means **operational rollback**, not on-chain undo.

| Scenario | Action |
|----------|--------|
| **Bad deploy, no nodes live** | Deploy fresh suite to new addresses; regenerate spec + genesis hash; re-sign; update all `CREG_*_ADDR` and `CREG_CHAIN_SPEC_URL`; do not reuse old genesis hash |
| **Wrong spec published** | Publish corrected `chain-spec.json` + `.sig`; restart nodes with `CREG_CHAIN_SPEC_OFFLINE=true` only if spec URL is down |
| **Node misconfigured** | Fix env; clear `CREG_DATA_DIR` only if you accept re-sync from genesis |
| **Partial CI failure** | Re-run workflow with `--resume` on forge verify step; inspect `contracts/broadcast/` for broadcast state |

Keep the previous `sepolia-latest.json` and spec in git history for forensics.

---

## Governance and feature flags

- On-chain governance is deployed, but the **node API returns HTTP 501** and the explorer hides governance until `VITE_GOVERNANCE_ENABLED=true` (Phase 2 — REM-202).
- Spec has `feature_flags.cross_chain: false` — align with [SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md](./SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md) decision **D4** before enabling bridge UI.

---

## Security reminders

- Rotate deployer/bridge/validator keys if exposed in logs or CI.
- Store `SPEC_SIGNING_KEY` only in CI secrets or HSM — never in the repo.
- Sepolia is public testnet; do not bridge real assets.
- See [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) for `CREG_DEV_SANDBOX`, `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM`, and production vs testnet rules.

---

## Related docs

- [SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md](./SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md) — REM-210, Phase 2
- [PHASE1_CLOSEOUT.md](./PHASE1_CLOSEOUT.md)
- [DATABASE_SCHEMA.md](./DATABASE_SCHEMA.md) — PostgreSQL mirror (optional on testnet)
- Local 3-validator Docker: `docker-compose.local-testnet.yml` + `local-testnet.ps1` (Anvil, not Sepolia)
