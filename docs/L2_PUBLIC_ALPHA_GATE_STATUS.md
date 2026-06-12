# L2 Public Alpha Gate Status

> Living tracker for [CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md](./CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md).  
> **Target level:** L2 Public Alpha (open waitlist, honest disclaimers — not mainnet).  
> **Last updated:** 2026-06-11

| Gate | Status | Evidence / next action |
| --- | --- | --- |
| Real sandbox on public validators (MAL-001) | **Partial** | Fleet verify passed with nsjail + `CREG_DEV_SANDBOX=false`; redeploy validators on post-merge image so `/v1/health` reports `sandbox.engine` |
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
| Public-alpha rehearsal (MAIN-006) | **Open** | Run `testnet/public-alpha-rehearsal.ps1` |
| Cloud Run hub-api (Phase 2) | **Pass** | `deploy-hub-api-cloudrun.ps1`; Phase 2 health — see [GCP_PHASE2_RUNBOOK.md](./GCP_PHASE2_RUNBOOK.md) |
| Public HTTPS LB (api.testnet.cregnet.dev) | **Partial** | LB `136.110.145.47`; backend HEALTHY; managed cert PROVISIONING/FAILED_NOT_VISIBLE until DNS cutover |
| Cloud Armor (WAF) | **Blocked** | `SECURITY_POLICIES` quota 0; `setup-cloud-armor.ps1` not applied |
| Observer pool (ILB) | **Partial** | ILB `10.128.0.5` provisioned; edge env not cut over — [GCP_PHASE2_RUNBOOK.md](./GCP_PHASE2_RUNBOOK.md) |
| DNS api.testnet.cregnet.dev | **Open** | A record `35.225.225.20` (edge VM); target LB `136.110.145.47` |

## Verify locally

From `chain-registry/`:

```powershell
.\testnet\l2-gate-verify.ps1
.\testnet\malicious-fixtures-verify.ps1
```

With live endpoints:

```powershell
.\testnet\l2-gate-verify.ps1 -Live -BaseDomain testnet.cregnet.dev
```

## Immediate operator actions

1. **SEC-401** — send outreach (Trail of Bits, OpenZeppelin); fill booking table in [NEXT_WORK.md](./NEXT_WORK.md).
2. **MAL-001** — `deploy-validator-fleet.ps1` after pulling `main` (includes `/v1/health` sandbox status).
3. **IPFS-002** — `.\testnet\gcp\setup-ipfs-pin-cron.ps1` (installs hourly cron + first evidence run).
4. **MAIN-006** — `.\testnet\public-alpha-rehearsal.ps1` when fleet + faucet are up.
5. **Phase 2** — observer pool cutover, DNS to LB `136.110.145.47`, Cloud Armor quota; see [GCP_PHASE2_RUNBOOK.md](./GCP_PHASE2_RUNBOOK.md).

