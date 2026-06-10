# Hybrid deployment — local validators, cloud edge

> **Updated:** 2026-06-10  
> **Note:** Production public testnet uses **Option A validator fleet** on GCP ([GCP-VALIDATOR-FLEET.md](./GCP-VALIDATOR-FLEET.md)). This doc is the legacy WireGuard-to-PC path.

> **Goal:** Run **creg-node-1/2/3** on your Windows PC; run **Caddy, explorer, faucet, IPFS, spec, waitlist** on GCP. Keep **self-hosted Sepolia** on `creg-sepolia-geth-vm` (no Infura).

## Architecture

```
                    Internet
                        │
                        ▼
              creg-testnet-vm (edge only, smaller CPU)
              Caddy TLS, explorer, faucet, IPFS, spec, waitlist
                        │
            WireGuard 10.200.0.0/24
                        │
                        ▼
              Your PC (10.200.0.2)
              creg-node-1/2/3 Docker
                        │
            WG route 10.128.0.0/9
                        ▼
              creg-sepolia-geth-vm :8545 (internal)
```

| Where | What |
|-------|------|
| **Local** | `creg-node-1`, `creg-node-2`, `creg-node-3` |
| **GCP testnet VM** | Caddy, explorer, faucet, IPFS, spec-server, waitlist static |
| **GCP Geth VM** | Sepolia JSON-RPC `http://10.128.0.3:8545` |

Public API `https://api.testnet.cregnet.dev` → Caddy → `10.200.0.2:28182` (local node-3).

## Cost impact

| Item | ~$/mo |
|------|-------|
| Before (all on cloud + Geth + NAT) | ~$220 |
| After (edge `e2-standard-2` + Geth + NAT) | ~**$165–180** |
| Firebase waitlist | ~$0–8 |

Downsize testnet VM after cutover: set `GCP_MACHINE_TYPE=e2-standard-2` in `hosting.env` and recreate or resize.

## One-time setup

### 1. WireGuard

Follow [testnet/gcp/wireguard/README.md](../chain-registry/testnet/gcp/wireguard/README.md).

### 2. Environment

In `testnet/sepolia-3node.env` (local + pushed to cloud for edge):

```env
CREG_HYBRID_MODE=true
CREG_WG_LOCAL_PEER=10.200.0.2
CREG_ETH_RPC=http://10.128.0.3:8545
SEPOLIA_RPC_URL=http://10.128.0.3:8545
CREG_CLOUD_IPFS_URL=https://ipfs.testnet.cregnet.dev
CREG_CLOUD_CHAIN_SPEC_URL=https://spec.testnet.cregnet.dev/chain-spec.json
```

Validator keys stay on your machine only (do not commit).

### 3. Cloud edge (GCP VM)

```powershell
.\testnet\gcp\push-env.ps1
.\testnet\gcp\sync-local-repo.ps1
.\testnet\gcp\ssh-vm.ps1 -Command "bash ~/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet/start-cloud-edge-gcp.sh"
```

Or set `CREG_HYBRID_MODE=true` in env and use updated `deploy-stack.ps1`.

### 4. Local validators (Windows)

```powershell
# WireGuard tunnel active first
.\testnet\start-local-validators.ps1
```

## Daily operations

| Task | Command |
|------|---------|
| Start local nodes | `.\testnet\start-local-validators.ps1` |
| Stop local nodes | `docker compose -f testnet/docker-compose.3node.yml -f testnet/docker-compose.local-hybrid.yml --env-file testnet/sepolia-3node.env down` |
| Cloud edge logs | `.\testnet\gcp\ssh-vm.ps1 -Command "docker logs -f creg-cloud-caddy"` |
| Public health | `curl https://api.testnet.cregnet.dev/v1/health` |

## Requirements

- PC online for validators — public chain API is down if local nodes + WireGuard are off.
- Windows Firewall: allow inbound **from 10.200.0.0/24** on TCP `28180–28182` (WireGuard only).
- Geth VM must stay up for L1 sync (`validator_set_sync`).

## Related

- [GCP-SEPOLIA-GETH-INTERNAL.md](./GCP-SEPOLIA-GETH-INTERNAL.md)
- [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md)
