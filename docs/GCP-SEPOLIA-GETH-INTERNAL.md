# Option A — Dedicated Sepolia Geth VM (internal RPC only)

> **Updated:** 2026-06-10  
> **Project:** `gen-lang-client-0022105784`  
> **Goal:** Replace public Infura/Alchemy with a **private** Sepolia JSON-RPC endpoint reachable only from `creg-testnet-vm`.

**Estimated cost:** ~**$55–69/mo** extra (`e2-standard-2` + 100 GB disk; default fits 250 GB regional SSD quota with `creg-testnet-vm`). See [GCP-RPC-ARCHITECTURE.md](./GCP-RPC-ARCHITECTURE.md).

---

## Architecture

```
Internet
   │
   ▼
creg-testnet-vm  (tag: creg-testnet)  — public 35.225.225.20, ports 22/80/443
   │  Docker: creg-node-1/2/3, Caddy, explorer, faucet…
   │  CREG_ETH_RPC=http://10.128.0.X:8545
   │
   │  VPC default (internal 10.128.0.0/9)
   ▼
creg-sepolia-geth-vm  (tag: creg-sepolia-geth)  — **no public IP**
   └── Geth Sepolia JSON-RPC :8545 (Docker)
```

| Resource | Exact name |
|----------|------------|
| Geth VM | `creg-sepolia-geth-vm` |
| Network tag (Geth) | `creg-sepolia-geth` |
| Network tag (testnet) | `creg-testnet` (existing) |
| Internal static IP | `creg-sepolia-geth-internal-ip` (region `us-central1`) |
| Firewall — RPC | `creg-sepolia-geth-allow-rpc-from-testnet` |
| Firewall — IAP SSH | `creg-sepolia-geth-allow-iap-ssh` |
| State file | `testnet/gcp/sepolia-geth-state.json` (gitignored) |

**No firewall rule** opens `8545` to `0.0.0.0/0`. The Geth VM is created with **`--no-address`** (no external IP).

**Cloud NAT** (`creg-nat-router` / `creg-nat` in `us-central1`) is required so the private VM can reach apt/Docker Hub for bootstrap. Provision creates it if missing (~$0–5/mo egress).

---

## Prerequisites

- `gcloud` authenticated; project `gen-lang-client-0022105784`
- `testnet/gcp/hosting.env` (copy from `hosting.env.example`)
- `creg-testnet-vm` already exists with tag `creg-testnet`

---

## 1. Provision Geth VM + firewall

```powershell
cd chain-registry
.\testnet\gcp\provision-sepolia-geth-vm.ps1 -Confirm
```

Creates:

- Internal static IP `creg-sepolia-geth-internal-ip`
- VM `creg-sepolia-geth-vm` (`e2-standard-2`, 100 GB boot disk, **no public IP**)
- Cloud NAT `creg-nat-router` / `creg-nat` if not already present
- Firewall: TCP **8545** from `source-tags=creg-testnet` → `target-tags=creg-sepolia-geth`
- Firewall: TCP **22** from IAP range `35.235.240.0/20` → `target-tags=creg-sepolia-geth`

---

## 2. Deploy Geth (Docker)

```powershell
.\testnet\gcp\deploy-sepolia-geth.ps1
```

Runs `ethereum/client-go` with `--sepolia`, HTTP RPC on `0.0.0.0:8545`. First sync can take **hours**.

Check sync from your workstation (via IAP):

```powershell
.\testnet\gcp\ssh-sepolia-geth.ps1 -Command "curl -s -X POST http://127.0.0.1:8545 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"eth_syncing\",\"id\":1}'"
```

When `eth_syncing` is `false`, the node is caught up.

---

## 3. Point validators at internal RPC

```powershell
.\testnet\gcp\get-sepolia-geth-rpc-url.ps1
```

Example output:

```text
SEPOLIA_RPC_URL=http://10.128.0.42:8545
CREG_ETH_RPC=http://10.128.0.42:8545
```

Add those lines to `testnet/sepolia-3node.env`, then redeploy the testnet stack:

```powershell
.\testnet\gcp\push-env.ps1
.\testnet\gcp\deploy-stack.ps1
```

Verify from **inside** the testnet VM (containers use host Docker network / VPC routing):

```powershell
.\testnet\gcp\ssh-vm.ps1 -Command "curl -s -X POST http://10.128.0.42:8545 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"eth_chainId\",\"id\":1}'"
```

Expect `"0xaa36a7"` (Sepolia `11155111`).

Then check CREG health:

```powershell
.\testnet\verify-rpc-endpoints.ps1
curl.exe -sS https://api.testnet.cregnet.dev/v1/health
# validator_set_sync.state should trend toward "synced"
```

---

## Firewall reference (manual gcloud)

If you need to recreate rules:

```bash
# RPC: only from testnet VM
gcloud compute firewall-rules create creg-sepolia-geth-allow-rpc-from-testnet \
  --project=gen-lang-client-0022105784 \
  --direction=INGRESS \
  --network=default \
  --action=ALLOW \
  --rules=tcp:8545 \
  --source-tags=creg-testnet \
  --target-tags=creg-sepolia-geth \
  --priority=1000

# SSH: IAP tunnel only (no public IP on Geth VM)
gcloud compute firewall-rules create creg-sepolia-geth-allow-iap-ssh \
  --project=gen-lang-client-0022105784 \
  --direction=INGRESS \
  --network=default \
  --action=ALLOW \
  --rules=tcp:22 \
  --source-ranges=35.235.240.0/20 \
  --target-tags=creg-sepolia-geth \
  --priority=1000
```

**Explicitly do not create** `0.0.0.0/0` → `8545`.

---

## SSH without a public IP

```powershell
.\testnet\gcp\ssh-sepolia-geth.ps1
```

Uses `gcloud compute ssh --tunnel-through-iap`.

---

## Tear down

```powershell
gcloud compute instances delete creg-sepolia-geth-vm --zone=us-central1-a --quiet
gcloud compute addresses delete creg-sepolia-geth-internal-ip --region=us-central1 --quiet
gcloud compute firewall-rules delete creg-sepolia-geth-allow-rpc-from-testnet --quiet
gcloud compute firewall-rules delete creg-sepolia-geth-allow-iap-ssh --quiet
```

Revert `CREG_ETH_RPC` in `sepolia-3node.env` to Infura/Alchemy and redeploy.

---

## Related

- [GCP-RPC-ARCHITECTURE.md](./GCP-RPC-ARCHITECTURE.md) — CREG API vs L1 RPC
- [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md) — monthly budget envelope
