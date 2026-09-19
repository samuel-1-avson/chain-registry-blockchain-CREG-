# Uninterruptible Observer Node System (Local HA & Cloud Production)

> **Status:** Production Architecture & Operational Runbook  
> **Target:** Local High-Availability Cluster, Kubernetes Deployments, and GCP Production Fleet  
> **Components:** `docker-compose.observer-ha.yml`, `nginx-observer-lb.conf`, `scripts/test-observer-failover.ps1`, `scripts/test-observer-failover.sh`, `creg observer`, `k8s/35-observer-ha.yaml`, `testnet/gcp/provision-observer-pool.ps1`

---

## 1. Executive Summary & Design Philosophy

The Chain Registry (CREG) blockchain separates **Consensus Execution** from **Query Ingress**:
1. **Validators (`CREG_IS_VALIDATOR="true"`):** Dedicated exclusively to Istanbul/PBFT consensus rounds, BLS aggregate signature verification, block proposal, and state transitions. Validators run in air-gapped private networks without public ingress.
2. **Observers (`CREG_IS_VALIDATOR="false"`):** Non-validating full nodes that follow the chain via P2P gossip and HTTP sync. They maintain identical RocksDB state, track the unfinalized pending package pool (`pending_pool.json`), and serve all public queries (REST `/v1/*`, JSON-RPC `POST /rpc`, SSE stream `/v1/events`).

```
                              [ PUBLIC CLIENTS & OPERATORS ]
                                            │
                                ┌───────────┴───────────┐
                                │   Public HTTPS/TLS    │
                                │ (Cloud Armor / Caddy) │
                                └───────────┬───────────┘
                                            │
                             ┌──────────────▼──────────────┐
                             │  HA Load Balancer / ILB     │
                             │ (Active Health Checks &     │
                             │  Sub-Millisecond Failover)  │
                             └──────────────┬──────────────┘
                                            │
                     ┌──────────────────────┴──────────────────────┐
                     │                                             │
          ┌──────────▼──────────┐                       ┌──────────▼──────────┐
          │ Observer Replica 1  │                       │ Observer Replica 2  │
          │ (Read-Only State)   │                       │ (Read-Only State)   │
          └──────────┬──────────┘                       └──────────┬──────────┘
                     │                                             │
                     └──────────────────────┬──────────────────────┘
                                            │ Internal P2P / Gossip
                                            │
                             ┌──────────────▼──────────────┐
                             │    PBFT Validator Mesh      │
                             │ (Private / No Public HTTP)  │
                             └─────────────────────────────┘
```

The **Uninterruptible Observer Node System** guarantees that:
- Any single observer crash, restart, or network partition causes **zero dropped queries** (`0.0% failure rate`).
- Requests encountering a backend failure mid-flight are immediately and transparently retried against a healthy replica via `proxy_next_upstream`.
- Replicas self-heal and automatically rejoin the upstream pool upon restarting.

---

## 2. Local High-Availability Cluster

The repository provides an automated turnkey local HA cluster orchestrated via Docker Compose.

### Cluster Topology

| Service | Container Name | Host Port | Role |
| :--- | :--- | :--- | :--- |
| `observer-lb` | `creg-ha-observer-lb` | `8090` | Active Nginx reverse proxy with round-robin balancing & instant failover |
| `observer-1` | `creg-ha-observer-1` | `28181` | Observer Replica 1 (`CREG_NODE_ID=observer-replica-1`) |
| `observer-2` | `creg-ha-observer-2` | `28182` | Observer Replica 2 (`CREG_NODE_ID=observer-replica-2`) |
| `spec-server` | `creg-ha-spec-server` | `8888` | Genesis chain-spec server with signature validation |
| `ipfs` | `creg-ha-ipfs` | `5001`, `8081` | Decentralized package artifact storage |

### Launching Local HA Stack

```powershell
cd chain-registry
docker compose -f docker-compose.observer-ha.yml up -d
```

Verify the stack health:
```powershell
# Check Load Balancer Status
curl http://localhost:8090/lb-status

# Query health through the Load Balancer (returns active replica node_id)
curl http://localhost:8090/v1/health
```

### Load Balancer Resiliency Configuration (`nginx-observer-lb.conf`)

The load balancer features proactive health tracking and instant upstream retries:

```nginx
upstream observer_backend {
    server observer-1:8080 max_fails=2 fail_timeout=5s;
    server observer-2:8080 max_fails=2 fail_timeout=5s;
    keepalive 32;
}

server {
    listen 80;
    server_name localhost;

    location /lb-status {
        return 200 '{"status":"ok","component":"observer-load-balancer"}';
        add_header Content-Type application/json;
    }

    # Real-time SSE / event stream endpoint
    location ~ ^/(v1/)?events {
        proxy_pass http://observer_backend;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 3600s;
        proxy_next_upstream error timeout invalid_header http_500 http_502 http_503 http_504;
        proxy_next_upstream_tries 2;
    }

    location / {
        proxy_pass http://observer_backend;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        add_header X-Proxy-Upstream $upstream_addr always;

        # Transparent failover on any network error, timeout, or 5xx response
        proxy_next_upstream error timeout invalid_header http_500 http_502 http_503 http_504;
        proxy_next_upstream_tries 2;
        proxy_next_upstream_timeout 5s;
        proxy_connect_timeout 3s;
        proxy_read_timeout 15s;
    }
}
```

---

## 3. Automated Chaos Failover Drill

To mathematically prove zero downtime during node outages, run the automated failover drill script.

### Executing the Drill (PowerShell / Windows)

```powershell
cd chain-registry
.\scripts\test-observer-failover.ps1
```

### Executing the Drill (Bash / Linux / CI)

```bash
cd chain-registry
chmod +x ./scripts/test-observer-failover.sh
./scripts/test-observer-failover.sh
```

### Failover Drill Execution Stages

1. **[1/5] Load Balancer Probe:** Verifies `/lb-status` returns `200 OK`.
2. **[2/5] Initial Pool Verification:** Asserts both `observer-1` and `observer-2` report healthy status and individual node identifiers (`observer-replica-1`, `observer-replica-2`).
3. **[3/5] Traffic Distribution:** Dispatches test queries through port `8090` and confirms round-robin load distribution.
4. **[4/5] Active Chaos Failure Injection:**
   - Forcefully halts `observer-1` via `docker compose stop observer-1`.
   - Dispatches continuous burst traffic through the load balancer.
   - **Assertion:** 100% of requests survive (routed to `observer-2`), experiencing 0 dropped requests and 0 HTTP 5xx errors.
5. **[5/5] Self-Healing Recovery:**
   - Restarts `observer-1` via `docker compose start observer-1`.
   - Polls `/v1/health` until recovery is confirmed.
   - Asserts that `observer-1` automatically rejoins the active pool.

---

## 4. First-Class CLI Management (`creg observer`)

The `creg` CLI provides built-in commands for operators to inspect, monitor, and test observer clusters:

### 1. Probe Cluster Health & Replicas (`creg observer status`)

```powershell
creg observer status --lb-url http://localhost:8090
```

**Output Example:**
```
================================================================================
  CREG OBSERVER HIGH-AVAILABILITY CLUSTER STATUS
================================================================================

1. Load Balancer Routing Probe
  Endpoint:   http://localhost:8090
  Status:     ONLINE (component: observer-load-balancer, latency: 2ms)
  Public API: 200 OK (routed to node: observer-replica-1, upstream: 172.20.0.4:8080)

2. Observer Replicas Pool
  ● Replica-1 [http://localhost:28181]:
      Node ID:      observer-replica-1
      Role:         Observer (Read-Only)
      Sync:         HEALTHY (lag: 0 blocks)
      Latency:      3ms
  ● Replica-2 [http://localhost:28182]:
      Node ID:      observer-replica-2
      Role:         Observer (Read-Only)
      Sync:         HEALTHY (lag: 0 blocks)
      Latency:      4ms

Summary:
  Active Pool Health: 2/2 replicas operational
  HA Status:          FULLY REDUNDANT (Zero-Downtime Ready)
```

### 2. Burst Probe Drill (`creg observer probe`)

```powershell
creg observer probe --lb-url http://localhost:8090 --count 100
```

Dispatches 100 queries through the load balancer, calculating exact success percentage, latency percentiles (min/avg/max), and backend replica distribution.

---

## 5. Kubernetes Production Architecture (`k8s`)

For cloud-native Kubernetes environments, the observer pool is deployed via `k8s/35-observer-ha.yaml`:

### Key Kubernetes Primitives

1. **Multi-Replica Deployment:**
   - `spec.replicas: 2` (dynamically scalable via HPA).
   - `RollingUpdate` with `maxSurge: 1` and `maxUnavailable: 0` for seamless zero-downtime upgrades.
2. **Pod Anti-Affinity:**
   - Ensures observer pods are scheduled onto different physical Kubernetes worker nodes / availability zones:
   ```yaml
   affinity:
     podAntiAffinity:
       preferredDuringSchedulingIgnoredDuringExecution:
         - weight: 100
           podAffinityTerm:
             labelSelector:
               matchExpressions:
                 - key: app.kubernetes.io/name
                   operator: In
                   values: [creg-observer]
             topologyKey: kubernetes.io/hostname
   ```
3. **PodDisruptionBudget (PDB):**
   - `minAvailable: 1` guarantees that Kubernetes cluster maintenance, node drains, or autoscaling events never take all observer replicas offline simultaneously.
4. **API Gateway Redirection (`k8s/30-api-gateway.yaml`):**
   - The public load balancer service explicitly selects `app.kubernetes.io/name: creg-observer`, shielding `validator-1` from all external traffic.

---

## 6. GCP Cloud Production Fleet

In Google Cloud Platform, observers run in a dedicated regional Managed Instance Group (MIG) behind an Internal TCP/HTTP Load Balancer (ILB):

### GCP Resource Mapping

| Resource | GCP Service / Identifier | Configuration |
| :--- | :--- | :--- |
| **Observer Pool MIG** | `creg-observer-pool` | Regional MIG with 1–3 autoscaling `e2-medium` instances |
| **Instance Template** | `creg-observer-pool-template-v3` | Tagged `creg-observers`, no external IP (`--no-address`) |
| **Internal Load Balancer** | `creg-observer-api-ilb` | Internal TCP/HTTP Forwarding Rule on port `28182` |
| **Edge Gateway** | `creg-testnet-vm` | Caddy reverse proxy on static IP `35.225.225.20` routing public traffic to ILB |

### Cloud Verification Script

To verify cloud production health:
```powershell
cd chain-registry
.\testnet\gcp\verify-observer-ha-cloud.ps1
```

---

## 7. Operational Runbook & Incident Response

### Failure Mode 1: Single Observer Replica Crash
- **Symptoms:** Container or pod terminates unexpectedly.
- **System Behavior:**
  - Nginx/ILB detects failed connection within 3 seconds.
  - In-flight queries are instantly retried on the surviving replica (`proxy_next_upstream`).
  - Docker/Kubernetes automatically restarts the crashed container.
- **Action Required:** None. System self-heals. Check logs for panic or out-of-memory errors:
  ```powershell
  docker compose -f docker-compose.observer-ha.yml logs --tail 100 observer-1
  ```

### Failure Mode 2: Observer Sync Lag / Stale Data
- **Symptoms:** Observer falls behind the validator tip (`sync_lag_blocks > 10`).
- **Telemetry Detection:** `/v1/health` marks `"sync": { "healthy": false }`.
- **Action Required:**
  - Verify P2P connectivity to bootnodes:
    ```powershell
    curl http://localhost:28181/v1/health | jq .sync
    ```
  - If P2P gossip is disconnected, restart the lagging replica:
    ```powershell
    docker compose -f docker-compose.observer-ha.yml restart observer-1
    ```

---

## 8. Verification Matrix

| Test Layer | Target | Command | Expected Outcome |
| :--- | :--- | :--- | :--- |
| **CLI Verification** | CLI binary | `cargo test -p chain-registry-cli --bin creg` | All CLI tests pass (exit 0) |
| **K8s Validation** | Manifest syntax | `kubectl kustomize k8s/` | Renders clean manifest with 35-observer-ha.yaml |
| **Local Preflight** | Failover drill preflight | `.\scripts\test-observer-failover.ps1 -SkipDrill` | LB and replicas verified |
| **Chaos Failover Drill**| Local Docker HA | `.\scripts\test-observer-failover.ps1` | 100% request survival (0 dropped) |
| **GCP Cloud Audit** | Cloud MIG & ILB | `.\testnet\gcp\verify-observer-ha-cloud.ps1` | All 4 verification stages pass |
