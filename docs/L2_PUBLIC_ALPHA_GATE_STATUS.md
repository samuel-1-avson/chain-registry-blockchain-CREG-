# L2 Public Alpha Gate Status

> Living tracker for [CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md](./CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md).  
> **Target level:** L2 Public Alpha (open waitlist, honest disclaimers — not mainnet).  
> **Last updated:** 2026-06-12

| Gate | Status | Evidence / next action |
| --- | --- | --- |
| Real sandbox on public validators (MAL-001) | **Pass** | Fleet verify: `engine=nsjail`, `CREG_DEV_SANDBOX=false`; public `GET https://api.testnet.cregnet.dev/v1/health` reports `sandbox.engine=nsjail` (2026-06-12) |
| Public endpoint health (HOSTING-301) | **Pass** | `hosting-301-verify.ps1` on `testnet.cregnet.dev` |
| Publisher quickstart E2E | **Pass** | `publisher-quickstart-verify.ps1` on `testnet.cregnet.dev`; release `v0.1.1-testnet` assets (2026-06-12) |
| Validator operator checklist (VAL-002) | **Pass** | [VALIDATOR_ONBOARDING_CHECKLIST.md](./VALIDATOR_ONBOARDING_CHECKLIST.md) |
| IPFS pinning + availability (IPFS-001/002) | **Pass** | Hourly cron on edge VM; `gcp/run-ipfs-pin-check.ps1`; reports in `testnet/ipfs-pin-logs/` |
| LLM advisory boundary visible (LLM-002) | **Pass** | Explorer `PackageIntelligencePanel`, CLI `output.rs`, hub `AlphaDisclaimer` + FAQ (merged PR #10) |
| Malicious fixture suite (MAL-002) | **Pass** | `testnet/malicious-fixtures/` + `malicious-fixtures-verify.ps1` + `cargo test -p validator malicious_fixture` |
| SEC-401 audit scheduled | **Open** | Scope ready; run `prepare-sec-401-outreach.ps1`, send RFPs, record vendor + start date |
| Incident response runbook | **Pass** | [INCIDENT_RESPONSE_RUNBOOK.md](./INCIDENT_RESPONSE_RUNBOOK.md) |
| Public copy — no production overclaims | **Pass** | In-repo review: [PUBLIC_COPY_REVIEW.md](./PUBLIC_COPY_REVIEW.md); hub disclaimers + quickstart alpha limits |
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

1. **SEC-401** — email [SEC-401-outreach-ready.md](./SEC-401-outreach-ready.md) to vendors; fill booking table in [NEXT_WORK.md](./NEXT_WORK.md).
2. **Cloud Armor** — `.\testnet\gcp\request-cloud-armor-quota.ps1` then `setup-cloud-armor.ps1` after quota grant.
3. **External marketing** — white paper + social (out of repo); keep alpha framing per [PUBLIC_COPY_REVIEW.md](./PUBLIC_COPY_REVIEW.md).
