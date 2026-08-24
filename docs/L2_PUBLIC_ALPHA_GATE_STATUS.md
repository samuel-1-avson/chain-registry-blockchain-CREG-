# L2 Public Alpha Gate Status

> Living tracker for the CREG limitations public readiness plan.  
> **Target level:** L2 Public Alpha (open waitlist, honest disclaimers — not mainnet).  
> **Last updated:** 2026-08-23 (banner); table rows below are the 2026-06-26 historical capture unless noted.

**2026-08-23 overlay:** the public GCP fleet is **paused**. Treat HOSTING-301 / public HTTPS LB / DNS “Pass” rows as historical evidence from when the fleet was up, not as a claim that `api.testnet.cregnet.dev` is reachable today. Canonical live status: [TESTNET_STATUS.md](./TESTNET_STATUS.md). Protocol shipped vs remaining: [PROTOCOL_PROGRESS.md](./PROTOCOL_PROGRESS.md). Software on `main` through PR #50 is coordinated public-alpha code; the live public network is waitlist-only until VAL-INDEPENDENCE and TESTNET-RESUME.

| Gate | Status | Evidence / next action |
| --- | --- | --- |
| Real sandbox on public validators (MAL-001) | **Pass** | Sandboxing verified on active nodes using nsjail isolation. |
| Public endpoint health (HOSTING-301) | **Pass** | Verified hostname responsiveness and HTTP redirects. |
| Publisher quickstart E2E | **Pass** | Verified CLI publishing flow end-to-end; release v0.1.1-testnet assets active. |
| Validator operator checklist (VAL-002) | **Pass** | Onboarding documentation complete. |
| IPFS pinning + availability (IPFS-001/002) | **Pass** | Automated replication active on public gateways. |
| LLM advisory boundary visible (LLM-002) | **Pass** | Interfaces updated to separate advisory summaries from consensus decisions. |
| Malicious fixture suite (MAL-002) | **Pass** | Evaluated security pipeline against threat behavior profiles. |
| SEC-401 audit scheduled | **Open** | Scope finalized; evidence pack pending ([WEAKNESSES_GAPS_REMEDIATION_PLAN.md](./WEAKNESSES_GAPS_REMEDIATION_PLAN.md) Phase 0). |
| Phase 1 remediation (gossip, batch, relays) | **Pass** | Iteration 5–6; forge 37/37; see [contracts/README.md](../chain-registry/contracts/README.md). |
| Maintainer next-steps doc | **Pass** | [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md) — testing, waitlist, Arena, validator onboarding. |
| Incident response runbook | **Pass** | Protocol incident response runbook drafted. |
| Public copy — no production overclaims | **Pass** | Project documentation reviewed for testnet bounding. |
| Waitlist segmentation | **Pass** | Waitlist functionality active. |
| Public-alpha rehearsal (MAIN-006) | **Pass** | End-to-end dry run verification passed. |
| Cloud Run hub-api (Phase 2) | **Pass** | Registry API deployed to hosting environment. |
| Public HTTPS LB (api.testnet.cregnet.dev) | **Pass** | Managed load balancer active; TLS certificate green. |
| Cloud Armor (WAF) | **Blocked** | Network firewall policies pending quota allocation. |
| Observer pool (ILB) | **Pass** | Observer node groups connected to private load balancer. |
| DNS api.testnet.cregnet.dev | **Pass** | Managed DNS settings configured. |
