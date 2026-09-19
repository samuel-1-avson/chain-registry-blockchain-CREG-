# GCP RPC architecture

> **Updated:** 2026-06-10  
> **Scope:** How Chain Registry API/JSON-RPC is exposed on GCP, what is **not** managed today, and a phased upgrade path with cost estimates.

Open the interactive view: **GCP RPC architecture** canvas in Cursor (beside this chat).

---

## RPC layers in this project

| Layer | Endpoint | Protocol | Hosted on GCP today? |
|-------|----------|----------|----------------------|
| **CREG node API** | `https://api.testnet.cregnet.dev` | REST `/v1/*`, JSON-RPC `POST /rpc`, SSE/WS | **Yes** — VM + Caddy |
| **CREG gRPC** | `:50051` (internal) | gRPC publish/watch | **No** public exposure |
| **Ethereum Sepolia** | `CREG_ETH_RPC` / `SEPOLIA_RPC_URL` | `eth_*` JSON-RPC | **No** — third-party provider |
| **Explorer wallet RPC** | `https://explorer.testnet.cregnet.dev/rpc` | Proxied to public Sepolia | **No** — forwards to publicnode |

### CREG JSON-RPC methods (live)

```bash
curl -sS https://api.testnet.cregnet.dev/v1/health

curl -sS -X POST https://api.testnet.cregnet.dev/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"creg_chainId","id":1}'
```

Implemented in `chain-registry/crates/node/src/json_rpc.rs`: `creg_chainId`, `creg_blockNumber`, `creg_getBlockByNumber`, `creg_health`.

### Ingress (today)

```
Client → api.testnet.cregnet.dev:443
      → Caddy (Caddyfile.fleet on edge VM)
      → creg-observer-pool:28182 (observer API)
```

See `chain-registry/testnet/caddy/Caddyfile`.

---

## What GCP runs today

| Resource | ID / value | Role |
|----------|------------|------|
| Compute Engine | `creg-testnet-vm`, `e2-standard-4` | Edge: Caddy + explorer + faucet + IPFS + spec |
| Compute Engine / MIG | `creg-observer-pool`, `e2-medium` | Public read observers following external validators |
| Compute Engine | `creg-sepolia-geth-vm`, `e2-standard-2` | Internal Sepolia JSON-RPC |
| Static IP | `35.225.225.20` | DNS for `*.testnet.cregnet.dev` |
| Firewall | TCP 22, 80, 443 | TLS on VM |
| Firebase (separate project) | `gen-lang-client-0098858574` | Waitlist only — **not** CREG RPC |

**Not used (by design — cost):** Cloud Load Balancing, GKE, Cloud Run, API Gateway, Blockchain Node Engine.

Budget context: [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md).

---

## Phased upgrade path

### Phase 0 — Today (done)

- Public HTTPS API on VM
- Verify: `chain-registry/testnet/verify-rpc-endpoints.ps1`

**Cost:** ~$122/mo (VM + disk + light egress)

### Phase 1 — Dedicated Sepolia RPC (QuickNode / Alchemy / Infura / Internal Geth)

Validators and `validator_set_sync` should not share a rate-limited public RPC.

> [!NOTE]
> **Google Cloud Blockchain RPC Status:** Google Cloud officially placed **Blockchain Node Engine** and **Blockchain RPC** into *limited support* on June 15, 2026, and scheduled full shutdown for December 15, 2026 (disabling new endpoint creation and removing `/blockchain-rpc` from the console). Google Cloud officially recommends migrating to **QuickNode** (Google Cloud Web3 partner).

Recommended options for dedicated Sepolia RPC:

1. **QuickNode (Google Cloud Recommended Partner)**:
   - Create a free Sepolia endpoint at [quicknode.com](https://www.quicknode.com)
   - Format: `https://[instance].ethereum-sepolia.quiknode.pro/[api_key]/`
2. **Alchemy / Infura**:
   - Create a free Sepolia app on [alchemy.com](https://www.alchemy.com) or [infura.io](https://www.infura.io)
   - Format: `https://eth-sepolia.g.alchemy.com/v2/YOUR_KEY` or `https://sepolia.infura.io/v3/YOUR_KEY`
3. **Internal Geth on GCP Compute Engine (Self-hosted Option)**:
   - See [GCP-SEPOLIA-GETH-INTERNAL.md](./GCP-SEPOLIA-GETH-INTERNAL.md)
   - No external API rate limits or third-party dependencies.
4. **Current Public Testnet Fallback**:
   - `https://ethereum-sepolia-rpc.publicnode.com` (free, already configured and working for deployment)

Configure your selected endpoint in `chain-registry/testnet/.env.sepolia` and `chain-registry/testnet/sepolia-fleet.env`:

```env
SEPOLIA_RPC_URL=https://...
CREG_ETH_RPC=https://...
```

Verify the endpoint before deploying:
```powershell
.\testnet\verify-sepolia-rpc.ps1
```

#### Phase 1b — Option A: self-hosted Geth (internal only)

No public JSON-RPC; GCP observers and faucet call a dedicated VM over the VPC. External validators use their own reachable Sepolia provider.

| Item | Value |
|------|--------|
| VM | `creg-sepolia-geth-vm` (`--no-address`) |
| Tag | `creg-sepolia-geth` |
| Firewall | `creg-sepolia-geth-allow-rpc-from-testnet` (tcp:8545, `source-tags=creg-testnet`) |
| `CREG_ETH_RPC` | `http://10.128.0.X:8545` (reserved internal IP) |

Runbook: [GCP-SEPOLIA-GETH-INTERNAL.md](./GCP-SEPOLIA-GETH-INTERNAL.md)

```powershell
.\testnet\gcp\provision-sepolia-geth-vm.ps1 -Confirm
.\testnet\gcp\deploy-sepolia-geth.ps1
.\testnet\gcp\get-sepolia-geth-rpc-url.ps1
```

**Cost:** ~+$69/mo (`e2-standard-2` + 100 GB disk).

#### Phase 1c — External validator operators

Validators run on independently controlled operator hosts; GCP does not provision or hold validator keys. The edge proxies public reads through the observer pool.

| Item | Value |
|------|--------|
| Endpoint source | Independently operated validators and bootnodes |
| Observer API | `CREG_OBSERVER_API_UPSTREAM` / internal observer ILB |
| Edge env | `CREG_VALIDATOR_P2P_SEEDS`, `CREG_VALIDATOR_HTTP_PEERS` (observer bootstrap only) |

Runbook: [GCP_PHASE2_RUNBOOK.md](./GCP_PHASE2_RUNBOOK.md)

The former GCP validator-fleet scripts are retained only as explicitly enabled
legacy lab tooling. The public deployment uses external operator endpoints and
the observer pool.

### Phase 2 — Global HTTPS load balancer (HA + DDoS option)

Use when you need static anycast IP, automated failover, or Cloud Armor — not required for alpha.

1. Reserve **global** static IP for the load balancer.
2. Create **instance group** with `creg-testnet-vm`.
3. Create **HTTPS load balancer**:
   - Backend: instance group, port 443 (or 8080 if terminating TLS on LB — prefer keeping Caddy on VM for simplicity).
   - Health check: `GET /v1/health` on `api.testnet.cregnet.dev` path or `:8080/v1/health` if checking node directly.
4. Point Cloudflare A record to **LB IP** (still grey-cloud if Caddy handles certs on VM; or orange-cloud / Google-managed certs if LB terminates TLS).

Sketch (Caddy stays on VM — LB passes TCP 443):

```bash
# Illustrative — adjust names/regions to match hosting.env
gcloud compute health-checks create https creg-api-health \
  --request-path=/v1/health --port=443

gcloud compute instance-groups unmanaged creg-testnet-ig \
  --zone=us-central1-a

gcloud compute instance-groups unmanaged add-instances creg-testnet-ig \
  --instances=creg-testnet-vm --zone=us-central1-a

gcloud compute backend-services create creg-api-backend --global \
  --protocol=HTTPS --health-checks=creg-api-health

# ... forwarding rule, URL map, target HTTPS proxy (see Google Cloud LB docs)
```

**Cost:** ~+$18–25/mo (forwarding rules + LB) + egress.

### Phase 3 — Read replica / second VM (scale)

- Add second VM with **observer or API-only** node behind LB.
- Keep **validators stateful** on known hosts — do not run PBFT + RocksDB on Cloud Run.

**Cost:** ~+$98/mo per `e2-standard-4`.

### gRPC publish path (optional)

- `creg` CLI can publish via gRPC when `--grpc-url` is set.
- Public internet: use **TCP load balancer** + mTLS, or VPN only.
- Not required if REST publish is sufficient.

---

## GCP services — fit summary

| GCP service | CREG REST/JSON-RPC | CREG gRPC | Sepolia eth_* |
|-------------|-------------------|-----------|---------------|
| **Blockchain RPC** | N/A (EVM only) | N/A | **Best fit** (managed EIP-1474, native to GCP) |
| **Compute Engine + Caddy** | Best fit today | Internal only | N/A |
| **External HTTPS LB** | Good for HA | No (use TCP LB) | N/A |
| **Cloud Run** | Poor (stateless) | Poor | N/A |
| **GKE** | Possible, high ops cost | Possible | Possible sidecar |
| **API Gateway** | REST subset only | No | No |
| **Blockchain Node Engine** | N/A | N/A | Dedicated node (higher ops/cost) |

---

## Verify live RPC

```powershell
cd chain-registry
.\testnet\verify-rpc-endpoints.ps1
.\testnet\verify-rpc-endpoints.ps1 -BaseUrl https://api.testnet.cregnet.dev
```

---

## Related docs

| Doc | Topic |
|-----|--------|
| [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md) | Two-project cost model |
| [GCP-VALIDATOR-FLEET.md](./GCP-VALIDATOR-FLEET.md) | Legacy validator-fleet lab (disabled by default) |
| [testnet/gcp/README.md](../chain-registry/testnet/gcp/README.md) | VM + Caddy deploy scripts |
| [OPERATOR.md](../chain-registry/testnet/OPERATOR.md) | External validators + GCP public services |
| [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | `CREG_NODE_URL` for developers |
