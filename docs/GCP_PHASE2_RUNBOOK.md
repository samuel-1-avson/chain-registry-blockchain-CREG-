# GCP Phase 2 runbook (observer pool + public API LB)

Operational guide for **Phase 2** testnet hosting on GCP: internal observer pool (ILB), **Cloud Run hub-api**, and the **public HTTPS load balancer** for `api.testnet.cregnet.dev`. Phase 1 (edge VM, validators, static CDN) remains on `creg-testnet-vm`.

**Last updated:** 2026-06-12

---

## Architecture snapshot

| Component | Address / name | Status (2026-06-12) |
|-----------|----------------|---------------------|
| Edge VM | `creg-testnet-vm` (`35.225.225.20` / `10.128.0.2`) | Caddy upstream `10.128.0.5:28182` (observer ILB) |
| Observer VM | `creg-observer-pool-j2q7` (`10.128.0.32`) | `creg-node:fleet`; `/v1/health` OK |
| Observer pool ILB | `10.128.0.5:28182` (internal) | Backend MIG attached; TCP health after LB probe firewall |
| Cloud Run hub-api | `https://creg-hub-api-wmkf4sobla-uc.a.run.app` | `GET /api/health` OK |
| Public HTTPS LB | `136.110.145.47` | **HTTP backend** on edge `:80` (LB terminates TLS); do not use HTTPS-to-HTTPS |
| Managed cert | `api.testnet.cregnet.dev` | ACTIVE on LB frontend |
| Cloud Armor | `setup-cloud-armor.ps1` | **Blocked** - project `SECURITY_POLICIES` quota = 0 |
| DNS | `api.testnet.cregnet.dev` | Cutover via `set-cloudflare-dns.ps1` → `136.110.145.47` (propagate 2–10 min) |

State files written by scripts (do not commit secrets):

- `chain-registry/testnet/gcp/public-lb-state.env` - LB IP and backend names
- `chain-registry/testnet/gcp/observer-pool-state.env` - ILB / MIG outputs (if present)

---

## Scripts

| Script | Purpose |
|--------|---------|
| `provision-observer-pool.ps1` | MIG + internal LB for observer nodes |
| `deploy-observer-pool.ps1` | Sync repo, push env, `start-observer-pool-gcp.sh` on pool VMs (`CREG_FLEET_BUILD=1` when image missing) |
| `start-observer-pool-gcp.sh` | Sources `sepolia-3node.env`; optional `CREG_FLEET_BUILD` / `CREG_OBSERVER_BUILD` |
| `build-export-node-image.sh` | On validator VM: compose build → tag `creg-node:fleet` → `/tmp/creg-node-fleet.tgz` |
| `import-observer-node-image.sh` | On observer VM: load tarball, restart observer container |
| `transfer-observer-node-image.ps1` | Validator → local → observer pipeline (use when observer disk is too small to build) |
| `recreate-caddy-observer-ilb.sh` | Force-recreate edge Caddy with ILB upstream (no full edge rebuild) |
| `set-cloudflare-dns.ps1` | Grey-cloud A records (e.g. `api` → public LB IP) |
| `deploy-hub-api-cloudrun.ps1` | Build/push/deploy hub-api to Cloud Run |
| `setup-gcp-public-lb.ps1` | Global HTTPS LB → edge instance group (named port `https`) |
| `setup-cloud-armor.ps1` | Attach Cloud Armor policy (requires quota) |

**Project / zone defaults:** `gen-lang-client-0022105784`, `us-central1-a`, IAP SSH/SCP.

---

## Env requirements (`sepolia-3node.env`)

- `CREG_OBSERVER_POOL_LB_IP=10.128.0.5`
- `CREG_OBSERVER_API_UPSTREAM=10.128.0.5:28182` (required for Caddy compose env interpolation)
- `CREG_HUB_API_CLOUD_RUN_URL` → Cloud Run hub-api URL

Push with `.\testnet\gcp\push-env.ps1 -VmName creg-testnet-vm -TunnelThroughIap` after edits.

---

## Recommended order

1. **Observer pool** - `deploy-observer-pool.ps1 -Confirm`; verify `curl http://127.0.0.1:28182/v1/health` on pool VM.
2. **ILB** - Confirm backend group on `creg-observer-api-backend` and `creg-observers-allow-lb-health-checks` firewall (probe ranges → `28182`).
3. **Edge** - `push-env` + `recreate-caddy-observer-ilb.sh` (or full `start-cloud-edge-gcp.sh`).
4. **Hub API** - `deploy-hub-api-cloudrun.ps1`; verify `https://testnet.cregnet.dev/api/health`.
5. **DNS** - `set-cloudflare-dns.ps1 -StaticIp 136.110.145.47 -RecordNames api`.
6. **Public LB cert** - Wait for managed cert ACTIVE on `136.110.145.47`.
7. **Cloud Armor** - After quota increase, `setup-cloud-armor.ps1`.

---

## Verification

```powershell
# Observer on VM
gcloud compute ssh creg-observer-pool-j2q7 --zone=us-central1-a --tunnel-through-iap --command="curl -fsS http://127.0.0.1:28182/v1/health"

# ILB from edge
gcloud compute ssh creg-testnet-vm --zone=us-central1-a --tunnel-through-iap --command="curl -fsS http://10.128.0.5:28182/v1/health"

# Public API (edge or after DNS)
curl.exe -fsS https://api.testnet.cregnet.dev/v1/health

gcloud compute backend-services get-health creg-observer-api-backend --region=us-central1
gcloud compute backend-services get-health creg-edge-api-backend --global
```

---

## Observer image pipeline

Observer pool VMs may have **~10 GB disk** — local `CREG_FLEET_BUILD` can fail with *no space left on device*. Build on the validator VM (49 GB) and transfer:

```powershell
# 1) On validator VM (after repo sync)
gcloud compute ssh creg-validator-vm --zone=us-central1-a --tunnel-through-iap `
  --command="bash ~/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet/gcp/build-export-node-image.sh"

# 2) From workstation — copies tarball validator → observer
.\testnet\gcp\transfer-observer-node-image.ps1 -Confirm

# 3) Import + restart on observer (or folded into transfer script)
gcloud compute ssh creg-observer-pool-j2q7 --zone=us-central1-a --tunnel-through-iap `
  --command="bash ~/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet/gcp/import-observer-node-image.sh"
```

`build-export-node-image.sh` tags **`CREG_FLEET_IMAGE`** / `ghcr.io/chain-registry/chain-registry:latest` (not a stale local `creg-node:fleet` tag). After import, verify public sandbox:

```powershell
(curl.exe -fsS https://api.testnet.cregnet.dev/v1/health | ConvertFrom-Json).sandbox
```

---

## Known issues / follow-ups

- **Provision idempotency:** If `creg-observer-api-backend` exists without backends, run `gcloud compute backend-services add-backend` or re-run an updated `provision-observer-pool.ps1`.
- **502 via public LB:** Classic external HTTPS LB with **HTTPS backend** to Caddy often 502s (requests never reach Caddy). Use **HTTP backend** on port 80; Caddy `Caddyfile.fleet` includes `http://{$CREG_PUBLIC_API_HOST}` for LB. Health check: `creg-edge-http-hc`.
- **TLS on public LB:** Managed cert on the **frontend** only; backend is plain HTTP to edge Caddy.
- **Cloud Armor:** Request `SECURITY_POLICIES` quota or run without WAF until approved.