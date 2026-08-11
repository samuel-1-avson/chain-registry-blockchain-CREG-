# GCP Phase 2 runbook (observer pool + public API LB)

Operational guide for **Phase 2** public-services hosting on GCP: internal observer pool (ILB), **Cloud Run hub-api**, and the **public HTTPS load balancer** for `api.testnet.cregnet.dev`. Validators are independently operated outside GCP.

**Last updated:** 2026-08-02

---

## Architecture snapshot

| Component | Address / name | Last recorded state (verify before rollout) |
|-----------|----------------|---------------------|
| Edge VM | `creg-testnet-vm` (`35.225.225.20` / `10.128.0.2`) | **TERMINATED** at last read; Caddy upstream must be reverified |
| Observer VM | `creg-observer-pool-j2q7` (`10.128.0.32`) | MIG target size **0** at last read; no active observer |
| Observer pool ILB | `10.128.0.5:28182` (internal) | Configured state must be verified after the MIG is resumed |
| Cloud Run hub-api | `https://creg-hub-api-wmkf4sobla-uc.a.run.app` | Service listed; verify `/api/health` |
| Public HTTPS LB | `136.110.145.47` | Configuration must be verified; backend should be HTTP edge `:80` |
| Managed cert | `api.testnet.cregnet.dev` | ACTIVE on LB frontend |
| Cloud Armor | `setup-cloud-armor.ps1` | **Blocked** - project `SECURITY_POLICIES` quota = 0 |
| DNS | `api.testnet.cregnet.dev` | Cutover via `set-cloudflare-dns.ps1` → `136.110.145.47` (propagate 2–10 min) |

The addresses and health observations above are historical evidence, not a
claim that the resources are currently running. Confirm instance, MIG, ILB,
certificate, and DNS state with the verification commands below before a
rollout.

State files written by scripts (do not commit secrets):

- `chain-registry/testnet/gcp/public-lb-state.env` - LB IP and backend names
- `chain-registry/testnet/gcp/observer-pool-state.env` - ILB / MIG outputs (if present)

---

## Scripts

| Script | Purpose |
|--------|---------|
| `provision-observer-pool.ps1` | MIG + internal LB for observer nodes |
| `deploy-observer-pool.ps1` | Sync repo, push env, start the immutable observer image; `-BuildFromSource` is rehearsal-only |
| `start-observer-pool-gcp.sh` | Sources `sepolia-fleet.env`; optional `CREG_FLEET_BUILD` / `CREG_OBSERVER_BUILD` |
| `build-export-node-image.sh` | Legacy lab image export; not required for the external-validator profile |
| `import-observer-node-image.sh` | On observer VM: load tarball, restart observer container |
| `transfer-observer-node-image.ps1` | Legacy validator-to-observer pipeline; prefer signed GHCR release images or observer-local build |
| `recreate-caddy-observer-ilb.sh` | Force-recreate edge Caddy with ILB upstream (no full edge rebuild) |
| `set-cloudflare-dns.ps1` | Grey-cloud A records (e.g. `api` → public LB IP) |
| `deploy-hub-api-cloudrun.ps1` | Build/push/deploy hub-api to Cloud Run |
| `setup-gcp-public-lb.ps1` | Global HTTPS LB → edge instance group (named port `https`) |
| `setup-cloud-armor.ps1` | Attach Cloud Armor policy (requires quota) |

**Project / zone defaults:** `gen-lang-client-0022105784`, `us-central1-a`, IAP SSH/SCP.

---

## Env requirements (`sepolia-fleet.env`)

- `CREG_OBSERVER_POOL_LB_IP=10.128.0.5`
- `CREG_OBSERVER_API_UPSTREAM=10.128.0.5:28182` (required for Caddy compose env interpolation)
- `CREG_HUB_API_CLOUD_RUN_URL` → Cloud Run hub-api URL
- `CREG_REQUIRE_SIGNED_IMAGES=true` and an immutable `CREG_FLEET_IMAGE=...@sha256:<digest>`
  for public observers. A source build is rehearsal-only and must be explicit.

Push with `.\testnet\gcp\push-env.ps1 -VmName creg-testnet-vm -TunnelThroughIap` after edits.

---

## Recommended order

1. **Preflight** - run `testnet/gcp/verify-public-services-profile.ps1 -Target observer` after filling the external validator/bootnode endpoints. The check is read-only and must pass before provisioning.
2. **Observer pool** - `deploy-observer-pool.ps1 -Confirm`; verify `curl http://127.0.0.1:28182/v1/health` on pool VM.
3. **ILB** - Confirm backend group on `creg-observer-api-backend` and `creg-observers-allow-lb-health-checks` firewall (probe ranges → `28182`).
4. **Edge preflight** - Add the ILB address to `sepolia-fleet.env`, then run `testnet/gcp/verify-public-services-profile.ps1` before `deploy-stack.ps1`.
5. **Edge** - `push-env` + `recreate-caddy-observer-ilb.sh` (or full `start-cloud-edge-gcp.sh`).
6. **Hub API** - `deploy-hub-api-cloudrun.ps1`; verify `https://testnet.cregnet.dev/api/health`.
7. **DNS** - `set-cloudflare-dns.ps1 -StaticIp 136.110.145.47 -RecordNames api`.
8. **Public LB cert** - Wait for managed cert ACTIVE on `136.110.145.47`.
9. **Cloud Armor** - After quota increase, `setup-cloud-armor.ps1`.

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

Observer pool VMs may have **~10 GB disk** — local `CREG_OBSERVER_BUILD` can fail with *no space left on device*. Prefer the signed GHCR release image. If a source build is required, build in CI or on a disposable build runner and publish its digest; do not use a validator VM as a build host:

```powershell
# Pull the signed release image on each observer VM, or build from the current
# source in a disposable CI runner and publish its digest before rollout.
gcloud compute ssh creg-observer-pool-j2q7 --zone=us-central1-a --tunnel-through-iap `
  --command="bash ~/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet/start-observer-pool-gcp.sh"
```

The legacy `build-export-node-image.sh` path is guarded and is not part of the
public rollout. After pulling the approved digest (or completing a reviewed
rehearsal source build), verify the observer sandbox/read-only profile:

```powershell
(curl.exe -fsS https://api.testnet.cregnet.dev/v1/health | ConvertFrom-Json).sandbox
```

---

## Known issues / follow-ups

- **Provision idempotency:** If `creg-observer-api-backend` exists without backends, run `gcloud compute backend-services add-backend` or re-run an updated `provision-observer-pool.ps1`.
- **502 via public LB:** Classic external HTTPS LB with **HTTPS backend** to Caddy often 502s (requests never reach Caddy). Use **HTTP backend** on port 80; Caddy `Caddyfile.fleet` includes `http://{$CREG_PUBLIC_API_HOST}` for LB. Health check: `creg-edge-http-hc`.
- **TLS on public LB:** Managed cert on the **frontend** only; backend is plain HTTP to edge Caddy.
- **Cloud Armor:** Request `SECURITY_POLICIES` quota or run without WAF until approved.
