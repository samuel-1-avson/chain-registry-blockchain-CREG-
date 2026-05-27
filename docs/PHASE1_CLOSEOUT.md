# Phase 1 closeout checklist

**Date:** 2026-05-27

## Verification


| Item              | Status | Notes                                                 |
| ----------------- | ------ | ----------------------------------------------------- |
| DOC-101–104       | Done   | See [docs/README.md](./README.md)                     |
| REM-101           | Done   | Explorer relayer client aligned                       |
| REM-102           | Done   | `chain-registry/migrations/`                          |
| SEC-104           | Done   | Grouped publisher/validator rate limits + unit tests  |
| SEC-102 / SEC-106 | Done   | Prod env fail-fast + `creg doctor`                    |
| SEC-201           | Done   | ZKVerifier: 6 passed (Foundry via Docker, 2026-05-27) |
| REM-201           | Done   | Governance 501 + explorer gated                       |
| REM-103           | Done   | JSON cursor sidecar + tests                           |
| Local smoke       | Done   | `local-testnet.ps1 -RunSmokeTests` (2026-05-27)      |


## Remaining before Phase 2

- ~~`.\local-testnet.ps1 -RunSmokeTests`~~ — **passed** 2026-05-27 (~2.5 min): gRPC on `30157–30160`; `creg doctor` all checks; package publish → PBFT inclusion (`tip_height=1`, `package_count=1`, `validator_set_sync_state=synced`).
- Open PR with Phase 1 diff; confirm CI `contracts` + `rust` jobs green on the branch
- Security ops runbook reviewed by a second reader

## Foundry (local, Windows + Docker)

```powershell
docker run --rm -v "${PWD}/chain-registry:/app" -w /app --entrypoint forge `
  ghcr.io/foundry-rs/foundry:stable test --match-contract ZKVerifier -vv `
  --out /tmp/forge-out --cache-path /tmp/forge-cache
```

Use `/tmp` output paths to avoid permission errors on bind-mounted `contracts/out`.