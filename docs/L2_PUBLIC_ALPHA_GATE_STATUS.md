# L2 Public Alpha Gate Status

> Living tracker for [CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md](./CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md).  
> **Target level:** L2 Public Alpha (open waitlist, honest disclaimers — not mainnet).  
> **Last updated:** 2026-06-12

| Gate | Status | Evidence / next action |
| --- | --- | --- |
| Real sandbox on public validators (MAL-001) | **Pass** | Fleet verify: `engine=nsjail`, `CREG_DEV_SANDBOX=false`; public `GET https://api.testnet.cregnet.dev/v1/health` reports `sandbox.engine=nsjail` (2026-06-12) |
| Public endpoint health (HOSTING-301) | **Pass** | `hosting-301-verify.ps1` on `testnet.cregnet.dev` |
| Publisher quickstart E2E | **Partial** | `PUBLIC_TESTNET_QUICKSTART.md` + publish smokes; needs rehearsal sign-off |
| Validator operator checklist (VAL-002) | **Pass** | [VALIDATOR_ONBOARDING_CHECKLIST.md](./VALIDATOR_ONBOARDING_CHECKLIST.md) |
| IPFS pinning + availability (IPFS-001/002) | **Pass** | Hourly cron on edge VM; `gcp/run-ipfs-pin-check.ps1`; reports in `testnet/ipfs-pin-logs/` |
| LLM advisory boundary visible (LLM-002) | **Pass** | Explorer `PackageIntelligencePanel`, CLI `output.rs`, hub `AlphaDisclaimer` + FAQ (merged PR #10) |
| Malicious fixture suite (MAL-002) | **Pass** | `testnet/malicious-fixtures/` + `malicious-fixtures-verify.ps1` + `cargo test -p validator malicious_fixture` |
| SEC-401 audit scheduled | **Open** | Scope ready; run `prepare-sec-401-outreach.ps1`, send RFPs, record vendor + start date |
| Incident response runbook | **Pass** | [INCIDENT_RESPONSE_RUNBOOK.md](./INCIDENT_RESPONSE_RUNBOOK.md) |
| Public copy — no production overclaims | **Partial** | Hub FAQ/network/disclaimer copy shipped; white paper + social still need review |
| Waitlist segmentation | **Pass** | Firebase waitlist deployed |
| Public-alpha rehearsal (MAIN-006) | **Pass** | `public-alpha-rehearsal.ps1 -Execute` — l2_gates, malicious_fixtures, hosting_verify, fleet_sandbox all PASS (2026-06-12) |
| Cloud Run hub-api (Phase 2) | **Pass** | `deploy-hub-api-cloudrun.ps1`; Phase 2 health — see [GCP_PHASE2_RUNBOOK.md](./GCP_PHASE2_RUNBOOK.md) |
| Public HTTPS LB (api.testnet.cregnet.dev) | **Pass** | DNS → `136.110.145.47`; managed cert ACTIVE; HTTP backend to edge `:80` — `GET /v1/health` OK |
| Cloud Armor (WAF) | **Blocked** | `SECURITY_POLICIES` quota 0; `setup-cloud-armor.ps1` not applied |
| Observer pool (ILB) | **Pass** | ILB `10.128.0.5`; edge `CREG_OBSERVER_API_UPSTREAM=10.128.0.5:28182` — [GCP_PHASE2_RUNBOOK.md](./GCP_PHASE2_RUNBOOK.md) |
| DNS api.testnet.cregnet.dev | **Pass** | Cloudflare A → `136.110.145.47` |

## Verify locally

From `chain-registry/`:

```powershell
.\testnet\l2-gate-verify.ps1
.\testnet\malicious-fixtures-verify.ps1
```

With live endpoints:

```powershell
.\testnet\l2-gate-verify.ps1 -Live -BaseDomain testnet.cregnet.dev
.\testnet\public-alpha-rehearsal.ps1 -Execute -BaseDomain testnet.cregnet.dev
```

## Immediate operator actions

1. **SEC-401** — send outreach (Trail of Bits, OpenZeppelin); fill booking table in [NEXT_WORK.md](./NEXT_WORK.md).
2. **Cloud Armor** — request `SECURITY_POLICIES` quota increase; then run `setup-cloud-armor.ps1`.
3. **Public copy** — review white paper + social for production overclaims before widening waitlist.
4. **Observer redeploy** — one-command image path: build on validator VM, transfer, import (see [GCP_PHASE2_RUNBOOK.md](./GCP_PHASE2_RUNBOOK.md#observer-image-pipeline)).
