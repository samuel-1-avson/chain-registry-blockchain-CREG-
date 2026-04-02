# Chain Registry Testnet Deployment Guide

> **Target:** 10-validator permissioned testnet  
> **Purpose:** Validate Phase 1 security fixes and consensus stability before mainnet beta  
> **Expected Runtime:** 7 days minimum for meaningful metrics  
> **Created:** 2026-04-01

---

## Table of Contents

1. [Overview](#overview)
2. [Prerequisites](#prerequisites)
3. [Architecture](#architecture)
4. [Step 1: Generate Validator Keys](#step-1-generate-validator-keys)
5. [Step 2: Deploy the Testnet](#step-2-deploy-the-testnet)
6. [Step 3: Verify Consensus Health](#step-3-verify-consensus-health)
7. [Step 4: Enable Monitoring](#step-4-enable-monitoring)
8. [Step 5: Run Stress Tests](#step-5-run-stress-tests)
9. [Step 6: Collect 7-Day Metrics](#step-6-collect-7-day-metrics)
10. [Troubleshooting](#troubleshooting)
11. [Cleanup](#cleanup)

---

## Overview

This guide deploys a **10-validator Chain Registry testnet** using Docker Compose. The testnet includes:

- **10 validator nodes** running PBFT consensus
- **1 shared IPFS node** for package storage
- **1 local Ethereum testnet** (Anvil) for smart contract anchoring
- **1 PostgreSQL mirror** for query-layer testing
- **Prometheus + Grafana** for real-time observability

### Why 10 Validators?

PBFT requires `⌊2n/3⌋ + 1` nodes to finalize blocks. With `n = 10`:
- **Quorum = 7** validators must agree
- **Fault tolerance = 3** Byzantine validators can be tolerated
- This is large enough to test real consensus dynamics but small enough to run on a single developer workstation.

---

## Prerequisites

| Requirement | Version | Notes |
|---|---|---|
| Docker | 24.0+ | With BuildKit enabled |
| Docker Compose | 2.20+ | Supports `condition: service_healthy` |
| Python | 3.9+ | For key generation and stress tests |
| RAM | 16 GB | 10 Rust nodes + IPFS + Anvil + Postgres |
| Disk | 50 GB free | Chain data, IPFS pins, build cache |
| OS | Linux / macOS / WSL2 | Windows native not recommended |

### Python Dependencies

```bash
pip install aiohttp cryptography
```

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Chain Registry Testnet                               │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐        ┌─────────┐  ┌─────────┐    │
│  │ node-1  │──│ node-2  │──│ node-3  │  ...   │ node-9  │──│ node-10 │    │
│  │ (seed)  │  │         │  │         │        │         │  │         │    │
│  └────┬────┘  └────┬────┘  └────┬────┘        └────┬────┘  └────┬────┘    │
│       │            │            │                  │            │         │
│       └────────────┴────────────┴──────────────────┴────────────┘         │
│                         libp2p Gossipsub + PBFT                            │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────┐         │
│  │  Shared Infrastructure                                        │         │
│  │  ┌──────┐  ┌───────┐  ┌──────────┐  ┌─────────────────┐     │         │
│  │  │ IPFS │  │ Anvil │  │ PostgreSQL│  │ Prometheus +    │     │         │
│  │  │      │  │ (L1)  │  │  (mirror) │  │ Grafana         │     │         │
│  │  └──────┘  └───────┘  └──────────┘  └─────────────────┘     │         │
│  └──────────────────────────────────────────────────────────────┘         │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Step 1: Generate Validator Keys

Each validator needs a unique Ed25519 keypair. We provide a script that generates keys deterministically and produces the `.env.testnet` file.

### Linux / macOS

```bash
cd chain-registry/chain-registry
python scripts/generate-testnet-keys.py --nodes 10 --output .env.testnet
```

### Windows (PowerShell)

```powershell
cd chain-registry/chain-registry
.\scripts\generate-testnet-keys.ps1 -Nodes 10 -Output .env.testnet
```

### Output

The script creates:
- `.env.testnet` — Docker Compose environment variables
- `config/validator-set.json` — Human-readable validator registry

Example `.env.testnet` (truncated):
```dotenv
CREG_ETH_RPC=http://localhost:8545
CREG_IPFS_URL=http://localhost:5001
CREG_PG_URL=postgres://creg:creg@localhost:5432/chain_registry

NODE1_VALIDATOR_KEY=a575d467...
NODE2_VALIDATOR_KEY=efd607a1...
...
NODE10_VALIDATOR_KEY=...

VALIDATOR_SET_JSON={"validators":[{"id":"node-1","alias":"Validator-1",...}]}

TESTNET_PUBLISHER_KEY=...
TESTNET_PUBLISHER_PUBKEY=...
```

> **Security note:** These are throwaway testnet keys. Never use them on mainnet.

---

## Step 2: Deploy the Testnet

### 2.1 Build and Start

```bash
docker compose -f docker-compose.testnet.yml --env-file .env.testnet up -d --build
```

This command:
1. Builds the `chain-registry-node` image
2. Starts IPFS, Anvil, PostgreSQL, and the contract deployer
3. Waits for Anvil to be healthy
4. Deploys all Solidity contracts
5. Starts validators `node-1` through `node-10`

### 2.2 Check Service Status

```bash
# List all containers
docker compose -f docker-compose.testnet.yml ps

# Expected output:
# NAME                    STATUS
# creg-testnet-anvil      healthy
# creg-testnet-ipfs       running
# creg-testnet-postgres   healthy
# creg-testnet-node-1     healthy
# ...
# creg-testnet-node-10    healthy
```

### 2.3 Port Reference

| Service | Host Port | Container Port |
|---|---|---|
| node-1 API | 8080 | 8080 |
| node-2 API | 8082 | 8080 |
| node-3 API | 8083 | 8080 |
| ... | ... | ... |
| node-10 API | 8090 | 8080 |
| IPFS API | 5001 | 5001 |
| IPFS Gateway | 8081 | 8080 |
| Anvil RPC | 8545 | 8545 |
| PostgreSQL | 5432 | 5432 |

---

## Step 3: Verify Consensus Health

### 3.1 Check All Nodes Are Reachable

```bash
for port in 8080 8082 8083 8084 8085 8086 8087 8088 8089 8090; do
  echo -n "Port $port: "
  curl -s http://localhost:$port/v1/health | jq -r '.status // "DOWN"'
done
```

### 3.2 Verify Chain Height is Advancing

```bash
# Watch chain height on node-1
watch -n 5 'curl -s http://localhost:8080/v1/chain/stats | jq .tip_height'
```

### 3.3 Publish a Test Package

```bash
# Create a dummy tarball
echo '{"name":"test-pkg","version":"1.0.0"}' > package.json
tar czf test-pkg.tgz package.json

# Upload to IPFS and get CID
CID=$(curl -s -X POST -F file=@test-pkg.tgz http://localhost:5001/api/v0/add | jq -r '.Hash')
echo "IPFS CID: $CID"

# Submit to node-1
curl -X POST http://localhost:8080/v1/packages \
  -H "Content-Type: application/json" \
  -d '{
    "id": {"ecosystem": "npm", "name": "test-pkg", "version": "1.0.0"},
    "content_hash": "'$(sha256sum test-pkg.tgz | awk '{print $1}')'",
    "ipfs_cid": "'$CID'",
    "publisher_pubkey": "'$(cat .env.testnet | grep TESTNET_PUBLISHER_PUBKEY | cut -d= -f2)'",
    "signature": "00" 
  }'
```

Wait 10–30 seconds, then verify:

```bash
curl -s http://localhost:8080/v1/packages/npm:test-pkg@1.0.0 | jq '.status'
# Expected: "verified"
```

---

## Step 4: Enable Monitoring

### 4.1 Start Prometheus + Grafana

```bash
docker compose \
  -f docker-compose.testnet.yml \
  -f observability/docker-compose.observability.yml \
  -f docker-compose.testnet.observability.yml \
  up -d
```

### 4.2 Access Dashboards

| Tool | URL | Default Credentials |
|---|---|---|
| Grafana | http://localhost:3000 | admin / admin |
| Prometheus | http://localhost:9090 | — |
| Alertmanager | http://localhost:9093 | — |

### 4.3 Key Metrics to Watch

| Metric | Query | Healthy Range |
|---|---|---|
| Chain height | `max(creg_chain_height)` | Increasing every 5s |
| Verified packages | `max(creg_package_count)` | Increases with stress test |
| Pending pool | `max(creg_pending_pool_size)` | < 20 |
| Nodes online | `count(up{job="chain_registry_testnet_nodes"} == 1)` | 10 |
| Consensus latency | (measured by stress test) | < 5s P95 |

### 4.4 Pre-configured Alerts

The testnet ships with Prometheus alerts in `observability/alerts.yml`:
- **ChainStalled** — no blocks in 5 minutes with pending packages
- **ChainNodeDown** — a validator is unreachable
- **PendingPoolBacklog** — > 50 packages stuck in pending
- **BlockHeightDivergence** — nodes have forked or partitioned

---

## Step 5: Run Stress Tests

The stress test publishes **N dummy packages** in parallel and measures how long consensus takes to verify each one.

### 5.1 Quick Test (100 packages)

```bash
python scripts/stress-test.py --nodes 10 --packages 100 --concurrency 20
```

### 5.2 Full Load Test (1,000 packages)

```bash
python scripts/stress-test.py --nodes 10 --packages 1000 --concurrency 50 --timeout 60
```

### 5.3 Expected Results

| Metric | Target | Acceptable |
|---|---|---|
| P50 consensus latency | < 3s | < 5s |
| P95 consensus latency | < 5s | < 10s |
| P99 consensus latency | < 10s | < 20s |
| Verification rate | > 95% | > 90% |
| Throughput | > 5 pkg/s | > 2 pkg/s |

### 5.4 Report Output

A JSON report is written to `stress-test-report.json`. Example summary:

```
============================================================
  Chain Registry Testnet Stress Test Report
============================================================
  Total packages submitted:     1000
  Accepted by API:              998
  Verified by consensus:        994
  Failed submissions:           2
  Timed out (>60s):             4
  Verification rate:            99.4%

  P50 consensus latency:        2100 ms
  P95 consensus latency:        4200 ms
  P99 consensus latency:        5800 ms
  Throughput:                   6.12 pkg/s
============================================================
```

---

## Step 6: Collect 7-Day Metrics

For the testnet to be considered stable, it must run continuously for **7 days** with the following success criteria:

### Success Criteria

| Check | Requirement |
|---|---|
| Uptime | All 10 nodes reachable > 99% of the time |
| Consensus | No `ChainStalled` alerts |
| Forks | No `BlockHeightDivergence` alerts > 5 blocks |
| Memory | No OOM kills on any node |
| Storage | Node disk usage growth < 1 GB/day |
| API | REST API P99 latency < 500 ms |

### Log Collection

```bash
# Export logs for all validators
docker compose -f docker-compose.testnet.yml logs > testnet-logs-$(date +%Y%m%d).txt

# Export Prometheus metrics
curl -s 'http://localhost:9090/api/v1/query?query=creg_chain_height[7d]' > chain-height-7d.json
```

---

## Troubleshooting

### Node Fails to Start

```bash
# Check logs
docker compose -f docker-compose.testnet.yml logs --tail 100 node-5

# Common causes:
# 1. Port collision — another service is using 8085
# 2. Invalid validator key — must be 64 hex chars
# 3. Contract deployment failed — check deploy-contracts logs
```

### Consensus Stalls

```bash
# Check if pending pool is growing
curl -s http://localhost:8080/v1/pending | jq

# Check validator set is identical on all nodes
for port in 8080 8082 8083 8084 8085 8086 8087 8088 8089 8090; do
  echo "=== Port $port ==="
  curl -s http://localhost:$port/v1/nodes | jq '.validators | length'
done
```

### IPFS Upload Fails

```bash
# Verify IPFS is reachable
curl -s http://localhost:5001/api/v0/id | jq '.ID'

# If IPFS is behind a firewall, ensure port 5001 is open
```

### High Memory Usage

Rust + libp2p + wasmtime can consume significant RAM. If nodes OOM:

```yaml
# In docker-compose.testnet.yml, add memory limits:
deploy:
  resources:
    limits:
      memory: 2G
```

---

## Cleanup

```bash
# Stop all testnet services
docker compose -f docker-compose.testnet.yml down

# Remove all data (irreversible)
docker compose -f docker-compose.testnet.yml down -v

# Remove built images
docker compose -f docker-compose.testnet.yml down --rmi all
```

---

## Next Steps After Testnet

1. **If 7-day testnet succeeds:** Proceed to external security audit (Phase 4)
2. **If consensus latency > 10s P95:** Optimize validator pipeline (parallel IPFS fetch, ZK fast-path)
3. **If nodes crash:** Debug and fix before Phase 2 feature work

---

*Document Version: 1.0*  
*Scope: 10-validator permissioned testnet for Phase 1 validation*
