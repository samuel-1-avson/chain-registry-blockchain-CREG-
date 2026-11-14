# Second CREG-run Sepolia node (Nethermind + Teku)

CREG reads Sepolia from nodes it operates rather than from RPC vendors. This is the second of three. It uses different client software from the first node (Geth + Lighthouse), so one client's bug or pruning gap is outvoted under quorum reads (`common::l1_rpc`, PRs #57 and #59).

| | Node A (existing) | **Node B (this)** | Node C (later) |
|---|---|---|---|
| VM | `creg-sepolia-rpc`, us-west1-b | `creg-sepolia-nethermind`, europe-west1-c, e2-standard-4, 34.79.38.130 / 10.96.1.3 | asia-southeast1 (430 GB SSD quota free) |
| Execution client | Geth | **Nethermind 2.2.2** | Reth or Erigon |
| Consensus client | Lighthouse | **Teku 26.10.3** | Nimbus or Lodestar |
| History | Full | From block 11,780,000 | From block 11,780,000 |

Both client versions are newer than the minimums for Sepolia's Glamsterdam fork (6 Oct 2026): Nethermind 2.1.0 and Teku 26.9.1.

## Files

| File | Purpose |
|---|---|
| `docker-compose.yml` | Reference definition of the two containers |
| `../gcp/provision-sepolia-nethermind.ps1` | Reserved IP, data disk, VM. **Dry run unless `-Apply`**; checks quota, subnet and firewall first |
| `../gcp/sepolia-nethermind-bootstrap.sh` | First boot: Docker, mount the data disk, generate the Engine API JWT secret on the VM |
| `../gcp/fetch-beacon-initial-state.ps1` | Copies the finalized beacon state from CREG's own Lighthouse to Teku, so no public checkpoint service is needed |
| `../gcp/deploy-sepolia-nethermind.ps1` | Starts both containers, pinned by digest |
| `../gcp/status-sepolia-nethermind.ps1` | Read-only status: genesis check, sync progress, peers, disk, recent warnings |
| `../gcp/_Invoke-RemoteBash.ps1` | Helper. Copies remote steps to the VM as a script file, because on Windows `gcloud` runs through `cmd.exe`, which mangles `--command` strings containing quotes or pipes |

On this laptop, PowerShell blocks scripts. Run each one with:

```
powershell -ExecutionPolicy Bypass -File <script>
```

## Network and access

- **Network:** the fleet VPC `creg-fleet-vpc`, subnet `creg-eu-west1` (10.96.1.0/24). Nothing new is created.
- **Firewall:** the tags `creg-fleet,creg-sepolia-rpc` apply the existing rules:
  - JSON-RPC 8545 from the fleet allowlist and 10.96.0.0/16;
  - Ethereum P2P on 30303 and 9000, open;
  - SSH through IAP only.
- **Local only:** the Engine API (8551) stays on the Docker network, and the Teku REST API (5051) is bound to localhost.
- **Credentials:** the VM has no service account.

## Sizing and cost

**Disk.** Nethermind measures a default Sepolia node at about 893 GB:
- 181 GB flat state;
- 548 GB block bodies;
- 132 GB receipts.

CREG needs receipts only from the R7 Staking deployment (block 11,781,426), so both ancient barriers are set to 11,780,000. With that cutoff, the expected use is roughly 250–350 GB, on a 380 GB disk.

> **Unverified:** the Nethermind docs name these settings, but don't say whether a recent block number like 11,780,000 is accepted. Check disk use partway through the sync. If the node is heading past about 350 GB, stop and decide: either keep full history (about 900 GB, which needs an SSD quota increase in europe-west1) or adjust the barrier.

**Quota.** europe-west1 has 430 GB of SSD free, and this needs 410 GB. us-west1 has 6 GB left, so **Geth's disk cannot grow further** without a quota increase.

**Cost.** About $140–170 a month at list price for an e2-standard-4 VM (4 vCPU / 16 GB), the disks and the reserved IPv4, before credits. The plan was n2-standard-4, but GCP had no n2 capacity in europe-west1-c on 2026-11-14. Network egress comes on top and depends on peer traffic. Peers are capped at 25 (execution) and 50 (consensus) to limit it. Check node A's actual egress in billing to calibrate.

## Steps

1. **Dry run** (read-only):
   ```
   powershell -ExecutionPolicy Bypass -File chain-registry\testnet\gcp\provision-sepolia-nethermind.ps1
   ```
2. **Provision** (creates billable resources):
   ```
   powershell -ExecutionPolicy Bypass -File chain-registry\testnet\gcp\provision-sepolia-nethermind.ps1 -Apply
   ```
   Wait for the startup script to finish. `/var/lib/creg-sepolia-nethermind-bootstrap.done` exists when it has.
3. **Teku start point from CREG's own Lighthouse:**
   ```
   powershell -ExecutionPolicy Bypass -File chain-registry\testnet\gcp\fetch-beacon-initial-state.ps1
   ```
   It records the slot, root and SHA-256 in `C:\Users\samue\.cargo-targets\sepolia-initial-state\initial-state-record.json`. While Geth is still syncing, Lighthouse reports the checkpoint as `execution_optimistic`. The checkpoint is network-finalized either way; re-check the root once Geth is at head.
4. **Deploy:**
   ```
   powershell -ExecutionPolicy Bypass -File chain-registry\testnet\gcp\deploy-sepolia-nethermind.ps1
   ```
   `-AllowExternalCheckpoint` exists only as a fallback, and is off by default.
5. **Watch the sync.** Use `eth_syncing` and `eth_blockNumber` on `127.0.0.1:8545`, `docker logs creg-sepolia-nethermind`, and `df -h /mnt/disks/sepolia-nethermind`.
6. **Acceptance against node A**, before any CREG node uses it:
   - same chain ID (`0xaa36a7`);
   - same block hash at a common finalized height;
   - identical normalised Staking logs from `0xb3c532` (11,781,426) to that height. This also proves the barrier left the needed history in place.

## Created 2026-11-14

- **21:18 UTC:** VM provisioned and first-boot setup done (Ubuntu 24.04, 373 GB data volume, JWT secret generated on the VM).
- **Teku start point:** taken from CREG's Lighthouse: slot 11,579,744, block root `0x4c10f275…`. The first start failed because the peer cap (50) was below Teku's default minimum (64); `--p2p-peer-lower-bound=40` fixed it. The partial database from that failed start was removed and Teku started again from the same file.
- **21:29 UTC:**
  - Nethermind returns the Sepolia genesis `0x25a5cc10…`. It follows the head (block 12,135,783, gas limit 200M) and is downloading state and history, with 8 peers.
  - Teku is at the head with 4 peers.
  - Teku and node A's Lighthouse report the same finalized root at slot 11,579,776 (`0xdbdcd0d4…`).
  - Disk use: 0.9 GB.

## Using it in the fleet

Two endpoints means both must agree on every quorum read, so one outage stops security-relevant reads. Until node C exists, choose one:

- **Node A alone, node B as standby** (recommended). Switch `CREG_ETH_RPC` to node B if Geth fails.
- **Both, 2-of-2.** Stricter, but reads stop if either node is down.

With node C running, set all three: `CREG_ETH_RPC` plus `CREG_ETH_RPC_FALLBACKS`, and the matching `FAUCET_*` and `RELAYER_*` settings. Inside the VPC, use the nodes' 10.96.x.x addresses.

## Removing it

The data disk is created with `auto-delete=no`. Removal takes three deletes: the instance (`gcloud compute instances delete creg-sepolia-nethermind --zone europe-west1-c`), then the disk (`creg-sepolia-nethermind-data`), then the address (`creg-sepolia-nethermind-ip`).
