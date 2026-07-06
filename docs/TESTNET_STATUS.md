# CREG testnet — live status

> **Last updated:** 2026-07-06  
> **Single source of truth** for what is running today. Supersedes stale “phase open” wording in older docs until those pages are refreshed.

---

## Summary

| Surface | Status | Notes |
|---------|--------|-------|
| **Public GCP fleet** (`api.testnet.cregnet.dev`, validators, IPFS gateway) | **Paused (cost)** | Fleet can stay off until external validators are ready |
| **Waitlist** | **Live** | Firebase — [WAITLIST-ONLY-HOSTING.md](./WAITLIST-ONLY-HOSTING.md) |
| **Contribution Arena** | **Live (alpha)** | Firebase + optional Hub API — [CREG-Contribution-Arena/README.md](../CREG-Contribution-Arena/README.md) |
| **Local / operator nodes** | **Available** | Run your own observer or validator against Sepolia chain spec |
| **Protocol code & docs** | **Active** | Public alpha; not mainnet |

---

## What “paused fleet” means

- Public REST at `https://api.testnet.cregnet.dev` may be **unreachable** while GCP hosting is stopped.
- Publishers and integrators should **not** assume the public API is online without checking health first.
- Operators can still run nodes locally or on their own infrastructure using [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) and [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md).

---

## Resume public hosting

When ready to bring the Sepolia fleet back:

```powershell
cd chain-registry\testnet\gcp
.\resume-testnet-hosting.ps1
```

See also [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md) Track A (system verification before resume).

---

## Related documents

| Document | Purpose |
|----------|---------|
| [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) | Alpha limits, package statuses, committee semantics |
| [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | Publisher / validator onboarding |
| [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md) | Maintainer planning hub |