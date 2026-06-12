# GCP Phase 2 runbook (observer pool + public API LB)

Operational guide for **Phase 2** testnet hosting on GCP: internal observer pool (ILB), **Cloud Run hub-api**, and the **public HTTPS load balancer** for `api.testnet.cregnet.dev`. Phase 1 (edge VM, validators, static CDN) remains on `creg-testnet-vm`.

**Last updated:** 2026-06-11

---

## Architecture snapshot

| Component | Address / name | Status (2026-06-11) |
|-----------|----------------|---------------------|
| Edge VM | `creg-testnet-vm` (`35.225.225.20`) | Caddy + legacy API path |
| Observer pool ILB | `10.128.0.5` (internal) | Provisioned; not wired in edge env yet |
| Cloud Run hub-api | Service per `hub-api-cloudrun-manifest.json` | Deployed; Phase 2 health OK |
| Public HTTPS LB | `136.110.145.47` | Provisioned; backend HEALTHY |
| Managed cert | `api.testnet.cregnet.dev` | PROVISIONING / FAILED_NOT_VISIBLE until DNS points at LB |
| Cloud Armor | `setup-cloud-armor.ps1` | **Blocked** — project `SECURITY_POLICIES` quota = 0 |
| DNS | `api.testnet.cregnet.dev` | Still A → `35.225.225.20` (edge), not `136.110.145.47` |

State files written by scripts (do not commit secrets):

- `chain-registry/testnet/gcp/public-lb-state.env` — LB IP and backend names
- `chain-registry/testnet/gcp/observer-pool-state.env` — ILB / MIG outputs (if present)

---

## Scripts (recovered from edge VM)

| Script | Purpose |
|--------|---------|
| `provision-observer-pool.ps1` | MIG + internal LB for observer nodes |
| `deploy-observer-pool.ps1` | Roll out observer pool images / config |
| `observer-pool-startup.sh` | Instance startup (GCE) |
| `docker-compose.observer-pool.yml` | Observer stack definition |
| `start-observer-pool-gcp.sh` | Operator helper on VM |
| `deploy-hub-api-cloudrun.ps1` | Build/push/deploy hub-api to Cloud Run |
| `hub-api-cloudrun-manifest.json` | Cloud Run service metadata |
| `setup-gcp-public-lb.ps1` | Global HTTPS LB → edge instance group (named port `https`) |
| `setup-cloud-armor.ps1` | Attach Cloud Armor policy (requires quota) |

**Project / zone defaults:** `gen-lang-client-0022105784`, `us-central1-a`, IAP SSH/SCP to `creg-testnet-vm`.

---

## Recommended order

1. **Observer pool** — `.\chain-registry\testnet\gcp\provision-observer-pool.ps1` then `deploy-observer-pool.ps1`; confirm ILB `10.128.0.5`.
2. **Hub API** — `.\chain-registry\testnet\gcp\deploy-hub-api-cloudrun.ps1`; verify Cloud Run URL / health.
3. **Public LB** — `.\chain-registry\testnet\gcp\setup-gcp-public-lb.ps1` (idempotent backend attach, `--named-ports="https:<port>"`, backend `--port-name=https`).
4. **DNS cutover** — Point `api.testnet.cregnet.dev` A record to `136.110.145.47`; wait for managed cert ACTIVE.
5. **Cloud Armor** — After quota increase, `.\chain-registry\testnet\gcp\setup-cloud-armor.ps1`.
6. **Edge env** — Point Caddy/hub-web at ILB / Cloud Run per `hub-edge.caddy.example` (no bogus `Host` to Cloud Run).

---

## Public LB script notes

`setup-gcp-public-lb.ps1` must use **named port** `https` on the unmanaged instance group and `--port-name=https` on the backend service so health checks hit Caddy HTTPS on the edge VM. Backend attachment is skipped if already present (`backendCount` guard).

---

## Verification

```powershell
# LB IP (from state file)
Get-Content chain-registry\testnet\gcp\public-lb-state.env

# Backend health (after gcloud auth)
gcloud compute backend-services get-health creg-edge-api-backend --global --project=gen-lang-client-0022105784

# DNS (should become LB IP after cutover)
Resolve-DnsName api.testnet.cregnet.dev
```

---

## Blockers / follow-ups

- **DNS:** Cut over `api.testnet.cregnet.dev` from edge VM to `136.110.145.47`.
- **TLS:** Managed certificate will not validate until DNS is correct.
- **Cloud Armor:** Request `SECURITY_POLICIES` quota or run without WAF until approved.
- **Observer cutover:** Wire edge `HUB_API` / observer RPC env to ILB `10.128.0.5` when ready.

