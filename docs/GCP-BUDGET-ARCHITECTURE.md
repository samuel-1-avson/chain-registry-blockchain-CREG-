# GCP & Firebase budget architecture

> **Updated:** 2026-06-09  
> **Scope:** Public Sepolia testnet (single VM) + marketing waitlist (static SPA + Firebase backend)

## Two GCP projects

| Project | ID | What runs here | Monthly target / cap |
|---------|-----|----------------|----------------------|
| **Testnet VM** | `gen-lang-client-0022105784` | Validators, Caddy TLS, explorer, faucet, IPFS, waitlist **static** nginx | **$150** / **$175** |
| **Waitlist backend** | `gen-lang-client-0098858574` | Firestore (named DB), `registerWaitlist` Cloud Function, Secret Manager (**Blaze**) | **$10** / **$25** |

**Typical combined spend:** ~$119 (VM) + ~$0–8 (Firebase at low signup volume) ≈ **~$122/month**.

## Architecture

```
Browser
  ├─ https://testnet.cregnet.dev/*     → VM 35.225.225.20 (project 0022105784)
  ├─ https://waitlist.cregnet.dev      → same VM (nginx SPA only)
  └─ Firebase SDK / registerWaitlist     → project 0098858574 (us-central1)
```

| Item | Value |
|------|--------|
| VM | `creg-testnet-vm`, `e2-standard-4`, `us-central1-a` |
| Static IP | `35.225.225.20` |
| Firestore database | `ai-studio-6b167dc8-a078-4526-a86b-de2a8722a753` |
| Callable function | `registerWaitlist` (Node 20, `us-central1`) |

## VM cost drivers (0022105784)

| Line item | Est. monthly |
|-----------|----------------|
| `e2-standard-4` on-demand | ~$98 |
| 100 GB `pd-balanced` disk | ~$10 |
| Egress (light–moderate) | ~$3–12 |
| Optional snapshots | ~$2–4 |

**Primary risk:** internet egress (Cloudflare DNS-only → all HTTPS/RPC bytes exit GCP).

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

- GKE, Cloud Load Balancing, Cloud SQL, second VM for validators
- Duplicating waitlist registration on the VM
