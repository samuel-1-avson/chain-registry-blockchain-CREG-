# Archived documentation

Historical and superseded documents. **Do not use for current operations.**

For current work, start at [../PROTOCOL_PROGRESS.md](../PROTOCOL_PROGRESS.md) and [../README.md](../README.md).

---

## Why files land here

- Superseded by [PROTOCOL_PROGRESS.md](../PROTOCOL_PROGRESS.md), [testnet/gcp/README.md](../../chain-registry/testnet/gcp/README.md), or remediation plan §6
- One-off cohort exports, copy-review checklists, or dated fix plans
- Duplicate outreach copies (canonical: [SEC-401-VENDOR-OUTREACH.md](./SEC-401-VENDOR-OUTREACH.md))
- Removed runbooks whose paths no longer exist in the tree
- Retired Sepolia 3-node lab (scale testnet is 10k+ external validators) — see [../../chain-registry/testnet/archive/3node-lab/](../../chain-registry/testnet/archive/3node-lab/)

---

## Archived in this directory

| File | Reason |
|------|--------|
| `CONSENSUS_FINALIZATION_FIX_PLAN.md` | Draft fix plan (2026-06-14); track outcomes in DEEP_DIVE + remediation plan |
| `CONTRACT_FIXES_REDEPLOY_PLAN.md` | Superseded by contract tests + Sepolia runbook |
| `genesis-alpha-wave1-ready.md` | Generated invite copy (2026-06-15) |
| `genesis-alpha-wave1-invites.md` | Generated invite copy |
| `PUBLIC_COPY_REVIEW.md` | One-off copy audit |
| `EXTERNAL_COPY_CHECKLIST.md` | One-off copy checklist |
| `SEC-401-outreach-ready.md` | Generated outreach; regenerate via `prepare-sec-401-outreach.ps1` |
| `WEB_APPS_DATABASE_API_AUDIT.md` | Point-in-time web audit snapshot |
| `HUB_API_REMEDIATION_STATUS.md` | Superseded by Hub design + Arena HUB_INTEGRATION |
| `GIT_PUSH.md` | Internal git note |
| `SECURITY_AND_REMEDIATION_IMPLEMENTATION_PLAN.md` | Early remediation plan |
| `PROJECT_ONBOARDING_LEGACY.md` | Legacy onboarding (pre-2026) |
| `DELIVERABLES_INDEX.md` | No copy in this tree; use `testnet/README.md` + `testnet/gcp/` scripts |
| `PACKAGE_INTELLIGENCE_LANE_C.md` | No copy in this tree (deferred feature spec) |
| `PUBLIC_TESTNET_DEPLOYMENT_AND_SOAK_RUNBOOK.md` | No copy in this tree; use `testnet/SOAK_TEST.md` + `PROTOCOL_PROGRESS.md` |
| `PUBLIC_TESTNET_INFRASTRUCTURE_CHECKLIST.md` | No copy in this tree; hosting scripts live under `testnet/` |
| `VALIDATOR_ONBOARDING_RUNBOOK_2026-05-18.md` | No copy in this tree; use `VALIDATOR_ONBOARDING_CHECKLIST.md` |
| `WAITLIST_DEEP_DIVE.md` | No copy in this tree; see `WAITLIST-ONLY-HOSTING.md` |
| `WAITLIST_FIREBASE_DEPLOY.md` | Historical copy; no active `docs/` copy remains — do not use for operations |
| `gcp-public-hosting.md` | No copy in this tree; GCP scripts live under `testnet/gcp/` (no README yet) |
| `testnet/spec-server/README.md` | Restored under `testnet/spec-server/` |
| `TESTNET_STATUS.md` | Former live fleet matrix (last update 2026-08-25); current status is observed inline in `WHITEPAPER.md` §19 and the root README |
| `TESTNET_PHASE_SCOPE.md` | Former participant-expectations page |
| `WHAT_TO_DO_NEXT.md` | Former maintainer planning hub; current briefing is `PROTOCOL_PROGRESS.md` |
| `NEXT_WORK.md` | Former prioritized ID list |
| `PUBLIC.md` | Former public documentation index; current index is `docs/README.md` |
| `TESTNET_SCALE_ARCHITECTURE.md` | Archived scale-topology design (not a deployment claim) |
| `TESTNET_SEPOLIA_RUNBOOK.md` | Archived Sepolia runbook |
| `DEEP_SYSTEM_ANALYSIS_AND_PROGRESS_REPORT.md` | Point-in-time system audit/scorecard |
| `DUAL_KEY_ARCHITECTURE_ENHANCEMENT_PROPOSAL.md` | Dual-key UX proposal |
| `RPC_ARCHITECTURE_AND_SYSTEM_REPORT.md` | Archived RPC/ingress report |
| `GCP-RPC-ARCHITECTURE.md` | Archived ingress cost/consolidation model |
| `REMEDIATION_BACKLOG.md` | Archived remediation backlog |
| `WEAKNESSES_GAPS_REMEDIATION_PLAN.md` | Archived gap-remediation plan |

---

## Deleted paths (no file — use replacement)

| Old path | Use instead |
|----------|-------------|
| `chain-registry/DELIVERABLES_INDEX.md` | [testnet/README.md](../../chain-registry/testnet/README.md), [testnet/gcp/](../../chain-registry/testnet/gcp/) scripts |
| `chain-registry/testnet/gcp-public-hosting.md` | [testnet/gcp/](../../chain-registry/testnet/gcp/) scripts (no README yet) |
| `chain-registry/testnet/FRIEND_ONBOARDING.md` | [VALIDATOR_ONBOARDING_CHECKLIST.md](../VALIDATOR_ONBOARDING_CHECKLIST.md) |
| `chain-registry/testnet/TAILSCALE_SETUP.md` | [HYBRID-LOCAL-VALIDATORS.md](./HYBRID-LOCAL-VALIDATORS.md) |

---

_Archived 2026-06-25/26; bulk archive pass 2026-09-19; index refreshed 2026-09-20._