# GCP & Firebase budget architecture

> **Updated:** 2026-06-10  
> **Scope:** Public Sepolia testnet (3-tier GCP VMs) + marketing waitlist (static SPA + Firebase backend)

## Two GCP projects

| Project | ID | What runs here | Monthly target / cap |
|---------|-----|----------------|----------------------|
| **Testnet hosting** | `gen-lang-client-0022105784` | Edge VM + validator fleet VM + internal Geth + NAT | **$300** / **$350** |
| **Waitlist backend** | `gen-lang-client-0098858574` | Firestore (named DB), `registerWaitlist` Cloud Function, Secret Manager (**Blaze**) | **$10** / **$25** |

**Typical combined spend (Option A, 3-node fleet):** ~$280–300 (GCP) + ~$0–8 (Firebase) ≈ **~$285–310/month**.

## Architecture (Option A — production)

```
Browser → creg-testnet-vm (edge, public IP)
       → creg-validator-vm (validators, private)
       → creg-sepolia-geth-vm (Sepolia RPC, private)
```

Runbook: [GCP-VALIDATOR-FLEET.md](./GCP-VALIDATOR-FLEET.md).

| Item | Value |
|------|--------|
| Edge VM | `creg-testnet-vm`, `e2-standard-4`, `us-central1-a` |
| Validator fleet VM | `creg-validator-vm`, `e2-standard-8`, no public IP |
| Geth VM | `creg-sepolia-geth-vm`, `e2-standard-2`, no public IP |
| Static IP (edge) | `35.225.225.20` |
| Firestore database | `ai-studio-6b167dc8-a078-4526-a86b-de2a8722a753` |
| Callable function | `registerWaitlist` (Node 20, `us-central1`) |

## VM cost drivers (0022105784, Option A)

| Line item | Est. monthly |
|-----------|----------------|
| `creg-testnet-vm` (`e2-standard-4`) | ~$98 |
| `creg-validator-vm` (`e2-standard-8`) | ~$196 |
| `creg-sepolia-geth-vm` (`e2-standard-2`) | ~$49 |
| Disks (100 + 200 + 100 GB `pd-balanced`) | ~$40 |
| Cloud NAT + egress | ~$5–15 |
| Optional snapshots | ~$2–8 |

**At 10 nodes:** same `e2-standard-8` validator VM; scale via compose only (~**$280–320/mo** total GCP).

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
4. VM kill switch: budget → Pub/Sub → stop `creg-testnet-vm` (see runbook in `testnet/gcp-public-hosting.md`)
5. Firebase emergency brake: disable `registerWaitlist` in Firebase console

## Deploy references

| Task | Script / doc |
|------|----------------|
| VM + testnet stack | [gcp-public-hosting.md](../chain-registry/testnet/gcp-public-hosting.md) |
| Waitlist static site | `chain-registry/testnet/gcp/deploy-waitlist.ps1` |
| Waitlist Firebase | [WAITLIST_FIREBASE_DEPLOY.md](./WAITLIST_FIREBASE_DEPLOY.md) |

## What we do not run (cost control)

- GKE, Cloud Load Balancing, Cloud SQL
- Duplicating waitlist registration on the VM

RPC/API ingress detail and phased LB options: [GCP-RPC-ARCHITECTURE.md](./GCP-RPC-ARCHITECTURE.md).
