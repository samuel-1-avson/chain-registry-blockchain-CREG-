# Option A — Dedicated validator fleet VM (VPC)

> **Updated:** 2026-06-10  
> **Project:** `gen-lang-client-0022105784`  
> **Goal:** Run CREG validators on a private GCP VM; edge TLS, explorer, faucet, IPFS, and spec stay on `creg-testnet-vm`.

Production path for the public Sepolia testnet. Hybrid WireGuard-to-PC remains documented in [HYBRID-LOCAL-VALIDATORS.md](./HYBRID-LOCAL-VALIDATORS.md) but is not the default rollout.

---

## Three-tier layout

```
Internet
   |
   v
creg-testnet-vm  (tag: creg-testnet)  -- public 35.225.225.20, ports 22/80/443
   |  Caddy, explorer, faucet, IPFS, spec-server, waitlist
   |  Proxies API/operator to validator fleet internal IP
   |
   |  VPC default (10.128.0.0/9)
   +---------------------------+---------------------------+
   |                           |                           |
   v                           v                           v
creg-validator-vm             creg-sepolia-geth-vm
(tag: creg-validators)          (tag: creg-sepolia-geth)
creg-node-1/2/3 Docker          Geth Sepolia :8545
API :28180-28189 (host)         (no public IP)
```

| Tier | VM | Tag | Public IP | Role |
|------|-----|-----|-----------|------|
| Edge | `creg-testnet-vm` | `creg-testnet` | Yes (static) | TLS ingress, shared services |
| Validator fleet | `creg-validator-vm` | `creg-validators` | **No** | `creg-node-1..N` (3 now, up to 10) |
| Sepolia RPC | `creg-sepolia-geth-vm` | `creg-sepolia-geth` | **No** | `CREG_ETH_RPC` for validators |

---

## Sizing

| Fleet size | Machine type | Boot disk | Est. VM cost/mo |
|------------|--------------|-----------|-----------------|
| 3 nodes (initial) | `e2-standard-8` | 200 GB | ~$216 |
| 10 nodes (target) | `e2-standard-8` | 200 GB | same VM (scale in compose) |

Combined with edge (`e2-standard-4` ~$108) + Geth (`e2-standard-2` ~$69) + NAT/egress (~$5-15):

| Stage | ~$/mo |
|-------|-------|
| 3-node fleet + edge + Geth | ~$280-300 |
| 10-node fleet (same VM) | ~$280-320 |

Detail: [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md).

---

## Port scheme (host on validator VM)

| Node | Role | API (host) | P2P (host) |
|------|------|------------|------------|
| 1 | validator (bootstrap) | 28180 | 29100 |
| 2 | validator | 28181 | 29101 |
| 3 | observer | 28182 | 29102 |
| 4-10 | reserved | 28183-28189 | 29103-29109 |

Edge Caddy and explorer proxy to **node 3** (public reads) and **node 2** (operator) on the validator internal IP.

Firewall from edge to fleet: TCP **28180-28199** (`creg-validators-allow-api-from-testnet`).

---

## Firewall summary

| Rule | Source | Target | Ports |
|------|--------|--------|-------|
| `creg-validators-allow-iap-ssh` | IAP `35.235.240.0/20` | `creg-validators` | 22 |
| `creg-validators-allow-api-from-testnet` | tag `creg-testnet` | `creg-validators` | 28180-28199 |
| `creg-testnet-allow-edge-from-validators` | tag `creg-validators` | `creg-testnet` | 15001, 18888 |
| `creg-sepolia-geth-allow-rpc-from-validators` | tag `creg-validators` | `creg-sepolia-geth` | 8545 |
| `creg-sepolia-geth-allow-rpc-from-testnet` | tag `creg-testnet` | `creg-sepolia-geth` | 8545 (existing) |

No rule opens validator or Geth ports to `0.0.0.0/0`.

**Cloud NAT** (`creg-nat-router` / `creg-nat`) is shared with the Geth VM so private VMs can pull Docker images.

---

## Required env vars

In `testnet/sepolia-3node.env` (after provision):

```env
CREG_VALIDATOR_FLEET_MODE=true
CREG_VALIDATOR_VM_INTERNAL_IP=10.128.0.X
CREG_EDGE_INTERNAL_IP=10.128.0.Y
CREG_ETH_RPC=http://10.128.0.3:8545
SEPOLIA_RPC_URL=http://10.128.0.3:8545
CREG_IPFS_URL=http://10.128.0.Y:15001
CREG_CHAIN_SPEC_URL=http://10.128.0.Y:18888/chain-spec.json
```

Get internal IPs:

```powershell
.\testnet\gcp\get-validator-fleet-internal-ip.ps1
gcloud compute instances describe creg-testnet-vm --zone=us-central1-a --format="get(networkInterfaces[0].networkIP)"
.\testnet\gcp\get-sepolia-geth-rpc-url.ps1
```

---

## Rollout steps

### 1. Provision validator VM + firewall

```powershell
cd chain-registry
.\testnet\gcp\provision-validator-vm.ps1 -Confirm
```

### 2. Deploy validator fleet (Docker)

Ensure `testnet/sepolia-3node.env` has validator keys, Geth RPC, and edge IP for IPFS/spec.

```powershell
.\testnet\gcp\deploy-validator-fleet.ps1
```

On the validator VM:

```bash
./testnet/start-validator-fleet-gcp.sh
```

### 3. Cut over edge (no validators on edge VM)

Set `CREG_VALIDATOR_FLEET_MODE=true` in `sepolia-3node.env`, then:

```powershell
.\testnet\gcp\push-env.ps1
.\testnet\gcp\sync-local-repo.ps1
.\testnet\gcp\ssh-vm.ps1 -Command "bash ~/creg-hosting/*/chain-registry/testnet/start-cloud-edge-gcp.sh"
```

Or full edge deploy:

```powershell
.\testnet\gcp\deploy-stack.ps1
```

(`start-remote-stack.sh` starts cloud-edge when `CREG_VALIDATOR_FLEET_MODE=true`.)

### 4. Verify

```powershell
.\testnet\verify-rpc-endpoints.ps1
.\testnet\gcp\ssh-validator-vm.ps1 -Command "docker ps"
```

---

## Scale to N nodes

```powershell
.\testnet\generate-validator-fleet-compose.ps1 -NodeCount 10
.\testnet\gcp\deploy-validator-fleet.ps1
```

Re-run `start-validator-fleet-gcp.sh` on the validator VM.

---

## Related docs

| Doc | Topic |
|-----|--------|
| [GCP-SEPOLIA-GETH-INTERNAL.md](./GCP-SEPOLIA-GETH-INTERNAL.md) | Internal Sepolia Geth |
| [GCP-RPC-ARCHITECTURE.md](./GCP-RPC-ARCHITECTURE.md) | API/RPC ingress |
| [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md) | Cost model |
| [HYBRID-LOCAL-VALIDATORS.md](./HYBRID-LOCAL-VALIDATORS.md) | Legacy PC + WireGuard path |
