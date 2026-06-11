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
| IPFS pinning + availability (IPFS-001/002) | **Partial** | `testnet/ipfs-pin-check.py` + `gcp/run-ipfs-pin-check.ps1`; schedule on edge VM + store reports in `testnet/ipfs-pin-logs/` |
| LLM advisory boundary visible (LLM-002) | **Partial** | Explorer `PackageIntelligencePanel`, CLI `output.rs`; hub copy still in progress |
| Malicious fixture suite (MAL-002) | **In progress** | `testnet/malicious-fixtures/` + `cargo test -p validator malicious_fixture` |
| SEC-401 audit scheduled | **Open** | Scope ready; run `prepare-sec-401-outreach.ps1`, send RFPs, record vendor + start date |
| Incident response runbook | **Pass** | [INCIDENT_RESPONSE_RUNBOOK.md](./INCIDENT_RESPONSE_RUNBOOK.md) |
| Public copy — no production overclaims | **Partial** | Hub FAQ/network pages; white paper + social still need review |
| Waitlist segmentation | **Pass** | Firebase waitlist deployed |
| Public-alpha rehearsal (MAIN-006) | **Open** | Run `testnet/public-alpha-rehearsal.ps1` |

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
3. **IPFS-002** — `.\testnet\gcp\run-ipfs-pin-check.ps1` and add hourly cron on edge VM.
4. **MAIN-006** — `.\testnet\public-alpha-rehearsal.ps1` when fleet + faucet are up.
