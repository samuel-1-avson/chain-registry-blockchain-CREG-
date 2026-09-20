# Chain Registry Testnet

Chain spec, bootstrap material, and operator guides for `creg-testnet-1`.
The checked-in Sepolia specification records four non-voting observer identities and two bootnodes on one GCP host;
this is a coordinated research alpha, not a BFT, decentralized, or public
scale-testnet deployment. [TESTNET_SCALE_ARCHITECTURE.md](../../docs/archive/TESTNET_SCALE_ARCHITECTURE.md) (archived 2026-09-19)
is future design only.

**Maintainers:** [docs/PROTOCOL_PROGRESS.md](../../docs/PROTOCOL_PROGRESS.md) (current briefing; [WHAT_TO_DO_NEXT.md](../../docs/archive/WHAT_TO_DO_NEXT.md) archived 2026-09-19)  
**Evidence snapshot:** [TESTNET_READINESS_REPORT.md](../TESTNET_READINESS_REPORT.md) (historical; see docs/PROTOCOL_PROGRESS.md for current phase)

---

## Quick links

| Doc | Audience |
|-----|----------|
| [QUICKSTART.md](./QUICKSTART.md) | Fastest path to a local node |
| [OPERATOR.md](./OPERATOR.md) | Sepolia validator fleet |
| [SOAK_TEST.md](./SOAK_TEST.md) | Resilience / load test plan |
| [gcp/](./gcp/) | GCP resume/stop/deploy scripts (no README yet) |
| [spec-server/README.md](./spec-server/README.md) | Signed chain-spec hosting |
| [bootstrap/README.md](./bootstrap/README.md) | Bootstrap node deployment |
| [../../docs/PUBLIC_TESTNET_QUICKSTART.md](../../docs/PUBLIC_TESTNET_QUICKSTART.md) | Publisher / developer / validator |
| [../../docs/archive/TESTNET_SEPOLIA_RUNBOOK.md](../../docs/archive/TESTNET_SEPOLIA_RUNBOOK.md) (archived 2026-09-19) | Sepolia deploy + spec publish |
| [../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md) | External validator onboarding |

---

## Operating models

| Model | Compose / script |
|-------|------------------|
| **External validator (primary)** | `../docker-compose.validator.yml` + [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md) |
| Maintainer seed fleet (GCP) | `docker-compose.validator-fleet.yml`, `start-validator-fleet-gcp.sh` |
| Sepolia bootstrap smoke | `../testnet.ps1`, `../docker-compose.testnet.yml` |
| Bootstrap host | `../docker-compose.testnet.yml`, `../testnet.ps1` |
| ~~3-node Sepolia lab~~ | **Retired** — [archive/3node-lab/](./archive/3node-lab/) |

---

## Verification

```bash
curl http://localhost:8080/v1/health
curl http://localhost:8080/v1/chain/stats | jq
creg testnet status --node-url http://localhost:8080
```

---

## Directory highlights

| Path | Purpose |
|------|---------|
| `chain-spec.sepolia.json` (+ `.sig`) | Signed canonical chain spec |
| `.env.sepolia.example` | Sepolia env template |
| `soak-test/` | Soak automation |
| `gcp/` | GCP hosting scripts |
| `spec-server/` | Static spec host for compose/fleet |

---

## Recommended flow

1. [QUICKSTART.md](./QUICKSTART.md) — local node  
2. [OPERATOR.md](./OPERATOR.md) — Sepolia fleet  
3. [SOAK_TEST.md](./SOAK_TEST.md) — after bootstrap is stable  
4. [gcp/](./gcp/) scripts (no README yet) — when resuming public hosting
