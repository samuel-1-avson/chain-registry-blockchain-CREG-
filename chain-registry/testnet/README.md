# Chain Registry Testnet

Chain spec, bootstrap material, and operator guides for `creg-testnet-1` — **10,000+ validator scale testnet** ([TESTNET_SCALE_ARCHITECTURE.md](../../docs/TESTNET_SCALE_ARCHITECTURE.md)).

**Maintainers:** [docs/WHAT_TO_DO_NEXT.md](../../docs/WHAT_TO_DO_NEXT.md)  
**Evidence snapshot:** [TESTNET_READINESS_REPORT.md](../TESTNET_READINESS_REPORT.md) (historical; see WHAT_TO_DO_NEXT for current phase)

---

## Quick links

| Doc | Audience |
|-----|----------|
| [QUICKSTART.md](./QUICKSTART.md) | Fastest path to a local node |
| [OPERATOR.md](./OPERATOR.md) | Sepolia validator fleet |
| [SOAK_TEST.md](./SOAK_TEST.md) | Resilience / load test plan |
| [gcp/README.md](./gcp/README.md) | GCP resume/stop/deploy scripts |
| [spec-server/README.md](./spec-server/README.md) | Signed chain-spec hosting |
| [bootstrap/README.md](./bootstrap/README.md) | Bootstrap node deployment |
| [../../docs/PUBLIC_TESTNET_QUICKSTART.md](../../docs/PUBLIC_TESTNET_QUICKSTART.md) | Publisher / developer / validator |
| [../../docs/TESTNET_SEPOLIA_RUNBOOK.md](../../docs/TESTNET_SEPOLIA_RUNBOOK.md) | Sepolia deploy + spec publish |
| [../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md) | External validator onboarding |

---

## Operating models

| Model | Compose / script |
|-------|------------------|
| **External validator (primary)** | `../docker-compose.validator.yml` + [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md) |
| Maintainer seed fleet (GCP) | `docker-compose.validator-fleet.yml`, `start-validator-fleet-gcp.sh` |
| Sepolia bootstrap smoke | `../testnet.ps1`, `../docker-compose.testnet.yml` |
| Bootstrap host | `../docker-compose.testnet.yml`, `../testnet.ps1` |
| ~~3-node Sepolia lab~~ | **Retired** — [archive/3node-lab/](./archive/3node-lab/README.md) |

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
4. [gcp/README.md](./gcp/README.md) — when resuming public hosting