# GCP & Firebase budget architecture

> **Updated:** 2026-06-10  
> **Scope:** Public Sepolia testnet (3-tier GCP VMs) + marketing waitlist (static SPA + Firebase backend)

## Two GCP projects

| Project | ID | What runs here | Monthly target / cap |
|---------|-----|----------------|----------------------|
| **Testnet hosting** | `gen-lang-client-0022105784` | Edge VM + observer pool + optional internal Geth + NAT | **$300** / **$350** |
| **Waitlist backend** | `gen-lang-client-0098858574` | Firestore (named DB), `registerWaitlist` Cloud Function, Secret Manager (**Blaze**) | **$10** / **$25** |

**Typical combined spend (maintainer edge + observers + optional Geth):** ~$180–250 (GCP) + ~$0–8 (Firebase) ≈ **~$185–260/month**. Validators and bootnodes run on independently controlled operator infrastructure — see [TESTNET_SCALE_ARCHITECTURE.md](./TESTNET_SCALE_ARCHITECTURE.md).

## Architecture Options

### Option 1: Unified Single-Instance Architecture (Recommended: ~$59/month)

Consolidates the entire testnet stack onto a single lean `e2-standard-2` VM running Caddy, CREG observer node, Explorer, and Faucet in lightweight containers. Upstream Sepolia L1 is proxied via Caddy to a free-tier provider ($0/mo).

```
Browser / Client ──► creg-testnet-vm (e2-standard-2, public IP 35.225.225.20)
                          │
                          ▼
                       Caddy (TLS)
                          ├──► /rpc, /v1/* ──► CREG Observer Node (:8080)
                          ├──► /sepolia   ──► Upstream Sepolia Proxy ($0/mo)
                          └──► /          ──► Explorer & Faucet (:80, :8082)
```

| Line item | Est. monthly |
|-----------|----------------|
| `creg-testnet-vm` (`e2-standard-2`, 2 vCPU, 8 GB RAM) | ~$49 |
| Boot disk (50 GB `pd-balanced`) | ~$5 |
| Static IP + light egress | ~$5 |
| Upstream Sepolia RPC (QuickNode/Alchemy free tier) | $0 |
| **Total Unified Stack Monthly Spend** | **~$59/month** |

Deploy script: `chain-registry/testnet/gcp/deploy-unified-stack.ps1`

---

### Option 2: Legacy 3-Tier Multi-VM Architecture (~$150–$160/month)

Separates edge routing from an observer managed instance group (MIG):

| Line item | Est. monthly |
|-----------|----------------|
| `creg-testnet-vm` (`e2-standard-4`) | ~$98 |
| Observer pool (`e2-medium`, one replica) | ~$25 |
| Optional `creg-sepolia-geth-vm` (`e2-standard-2`) | ~$49 |
| Disks (100 + 200 + 100 GB `pd-balanced`) | ~$40 |
| Cloud NAT + egress | ~$5–15 |
| Optional snapshots | ~$2–8 |

**At 10,000+ nodes:** validator compute and bandwidth are borne by independent operators; GCP scales only observers and public read services.

**Primary risk:** internet egress on the edge VM (Cloudflare DNS-only → HTTPS/RPC bytes exit GCP).

## Firebase cost drivers (0098858574)

| Line item | Low traffic | Viral spike |
|-----------|-------------|-------------|
| Firestore reads/writes | $0–2 | $15–40 |
| Cloud Functions Gen2 | $0–1 | $5–20 |
| Secret Manager + logging | &lt;$2 | &lt;$5 |

**Controls:** hardened Firestore rules (no client writes), server-side reCAPTCHA, separate GCP Budget at $25.

## Billing guardrails

1. Create GCP Budget **`creg-testnet-hosting`** — $175 on `0022105784`
2. Create GCP Budget **`creg-waitlist-firebase`** — $25 on `0098858574`
3. Enable billing export to BigQuery (optional)
4. VM kill switch: budget → Pub/Sub → stop `creg-testnet-vm` (see [testnet/gcp/README.md](../chain-registry/testnet/gcp/README.md))
5. Firebase emergency brake: disable `registerWaitlist` in Firebase console

## Deploy references

| Task | Script / doc |
|------|----------------|
| VM + testnet stack | [testnet/gcp/README.md](../chain-registry/testnet/gcp/README.md) |
| Waitlist static site | `chain-registry/testnet/gcp/deploy-waitlist.ps1` |
| Waitlist Firebase | [WAITLIST_FIREBASE_DEPLOY.md](./WAITLIST_FIREBASE_DEPLOY.md) |

## What we do not run (cost control)

- GKE, Cloud Load Balancing, Cloud SQL
- Duplicating waitlist registration on the VM

RPC/API ingress detail and phased LB options: [GCP-RPC-ARCHITECTURE.md](./GCP-RPC-ARCHITECTURE.md).
