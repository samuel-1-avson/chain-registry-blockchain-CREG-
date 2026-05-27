# Phase 2 kickoff — Sepolia first (Option A)

**Goal:** Prove the runbook, publish a signed chain spec, boot at least one node against Sepolia L1.

**Prerequisite:** Phase 1 PR opened (draft OK). Do not commit `.env.sepolia`.

---

## Already deployed?

`testnet/chain-spec.sepolia.json` may already list Sepolia contract addresses and a `genesis_hash`. If those contracts are live on Sepolia and you control the deployer keys:

| Path | When |
|------|------|
| **Reuse** | Skip redeploy; verify bytecode on Etherscan; publish spec + sig; boot node with `file://` or spec server |
| **Fresh deploy** | New addresses, new `genesis_hash`, re-sign spec, update all operator env |

If unsure, treat as **fresh deploy** (cleanest for operators).

---

## Step 1 — Create deploy secrets file

From `chain-registry/`:

```powershell
copy testnet\.env.sepolia.example testnet\.env.sepolia
```

Edit `testnet/.env.sepolia` and set at minimum:

```env
SEPOLIA_RPC_URL=https://sepolia.infura.io/v3/YOUR_KEY
DEPLOYER_KEY=0xYOUR_FRESH_SEPOLIA_KEY
ETHERSCAN_API_KEY=              # optional verify
GOVERNANCE_THRESHOLD=2
CREG_BRIDGE_KEY=0x...           # optional; can match deployer for solo test
```

Generate a deployer (never use Anvil default):

```powershell
docker run --rm ghcr.io/foundry-rs/foundry:stable cast wallet new
```

Fund the printed address on Sepolia (faucets in runbook).

---

## Step 2 — Deploy contracts (Docker; no local `forge` required)

```powershell
cd f:\project\chain-registry\chain-registry

# Load .env.sepolia into process (deploy script does this automatically)
.\testnet\deploy-sepolia.ps1
```

If `forge` is not on PATH, use Docker instead:

```powershell
$envFile = "testnet\.env.sepolia"
Get-Content $envFile | ForEach-Object {
  if ($_ -match '^\s*([^#\s][^=]*)\s*=\s*(.*)\s*$') {
    Set-Item -Path "env:$($matches[1])" -Value $matches[2]
  }
}

docker run --rm -v "${PWD}:/app" -w /app `
  -e SEPOLIA_RPC_URL -e DEPLOYER_KEY -e ETHERSCAN_API_KEY `
  ghcr.io/foundry-rs/foundry:stable `
  script contracts/script/DeploySepolia.s.sol:DeploySepolia `
  --rpc-url $env:SEPOLIA_RPC_URL `
  --private-key $env:DEPLOYER_KEY `
  --broadcast --chain-id 11155111 -vvv `
  --out /tmp/forge-out --cache-path /tmp/forge-cache
```

Manifest: `contracts/deployments/sepolia-latest.json`

---

## Step 3 — Finalize and sign chain spec

```powershell
.\testnet\finalize-sepolia-spec.ps1
```

Uses your **spec signing key** (prefer env var / CI secret, not a committed dev key).

Outputs:

- `testnet/chain-spec.sepolia.json` (updated addresses + `genesis_hash`)
- `testnet/chain-spec.sepolia.json.sig`

Verify:

```powershell
cargo run --example verify_chain_spec --package common -- `
  testnet/chain-spec.sepolia.json (Get-Content testnet/chain-spec.sepolia.json.sig)
```

---

## Step 4 — Publish spec (pick one)

**Local dev server**

```powershell
copy testnet\chain-spec.sepolia.json testnet\spec-server\
copy testnet\chain-spec.sepolia.json.sig testnet\spec-server\
cd testnet\spec-server
docker compose up -d
# CREG_CHAIN_SPEC_URL=http://localhost:8888/chain-spec.json
```

**GitHub Actions:** workflow `Sepolia Deploy` with `publish_spec: true`.

---

## Step 5 — Boot one validator

```powershell
# Ed25519 validator key: cargo run -p chain-registry-cli -- keygen
.\testnet\start-local-node.ps1 -Validator -ValidatorKey <64_hex_ed25519>
```

Or fill `testnet/.env.sepolia` node fields and run from your host script.

Checks:

```powershell
curl http://localhost:8080/v1/health
curl http://localhost:8080/v1/chain/stats
cargo run -p chain-registry-cli -- doctor --testnet
```

---

## Step 6 — Phase 2 exit proof (same week)

| Check | Command / artifact |
|-------|-------------------|
| Runbook exercised | Second person repeats Steps 1–5 or documents deltas |
| L1 contracts | Etherscan links for `staking`, `registry`, `zk_verifier` |
| Spec signature | `verify_chain_spec` exit 0 |
| Sync cursor restart | Stop node → restart → `validator_set_sync_state` still `synced`, no duplicate validators |
| Observability | REM-211 after metrics endpoint is up |

---

## After Sepolia (parallel Phase 2 code)

- SEC-203 — `creg chain-spec validate`
- SEC-101 / SEC-101b — hot-key runbook + startup warnings
- SEC-105 — Ed25519 → ETH address warning
- REM-203 — unify alloy
- REM-211 — Grafana/Prometheus vs testnet profile

Governance: keep **disabled** (`VITE_GOVERNANCE_ENABLED` unset) until REM-202 is explicitly scheduled (**D3**).

---

## Related docs

- [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md)
- [PHASE1_CLOSEOUT.md](./PHASE1_CLOSEOUT.md)
- [SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md](./SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md) § Phase 2
