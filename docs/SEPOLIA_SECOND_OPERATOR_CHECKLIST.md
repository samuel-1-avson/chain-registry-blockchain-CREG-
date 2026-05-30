# Sepolia Option A — second operator checklist

Use this when a second person repeats the reuse path to close the Phase 2 ops gap ([PHASE2_CLOSEOUT.md](./PHASE2_CLOSEOUT.md)).

**Prerequisites:** Docker (or Python for spec-server fallback), Rust toolchain, Sepolia RPC access, spec signing pubkey (from operator who published the spec).

## Checklist

| Step | Action | Pass? | Notes / deltas |
|------|--------|-------|----------------|
| 1 | Clone repo, checkout branch with Phase 2 ship | ☑ | Branch: `main` (Phase 2 + Phase 3 ship) |
| 2 | `cd chain-registry` | ☑ | Workspace crate root |
| 3 | `cargo build --bin creg-node -p chain-registry-node` (+ `creg` CLI) | ☑ | Built 2026-05-29 (`target/debug/`) |
| 4 | Set `CREG_SPEC_SIGNING_PUBKEY` (from spec publisher) | ☑ | Set by `run-sepolia-reuse.ps1` from `chain-spec.sepolia.json` |
| 5 | `.\testnet\run-sepolia-reuse.ps1` (no `-StartNode`) | ☑ | Python :8888 fallback (Docker Desktop down) |
| 6 | `.\testnet\run-ops-201-verify.ps1 -SkipPublish` | ☑ | `ops-201-20260529-225850.log` |
| 7 | `Invoke-RestMethod http://localhost:8090/v1/health` | ☑ | `synced` in ~20.4s |
| 8 | `creg chain-spec validate testnet/chain-spec.sepolia.json` | ☑ | exit 0 |
| 9 | Stop node, restart with same `sepolia-node-data` | ☑ | Resync ~11.8s |
| 10 | Record operator name, date, RPC URL used | ☐ | See sign-off below |

## Automated helper (recommended)

```powershell
cd chain-registry
cargo build --bin creg-node -p chain-registry-node
cargo build --bin creg -p chain-registry-cli
.\testnet\run-ops-201-verify.ps1 -SkipPublish -Force
```

Covers checklist steps 3–9 (spec server, node health `synced`, `chain-spec validate`, restart timing). Logs: `testnet/ops-201-logs/`. Results JSON: `ops-201-results-*.json`.

**Docker down?** `run-ops-201-verify.ps1` falls back to `python -m http.server` in `testnet/spec-server/`.

**Observer mode:** `CREG_IS_VALIDATOR=false` (do not enable validator mode without `CREG_VALIDATOR_KEY`).

**Publish smoke (optional):** After rebuilding `creg-node` with the observer pending-pool fix, follow [TESTNET_SEPOLIA_RUNBOOK.md § E2E-301](./TESTNET_SEPOLIA_RUNBOOK.md#publish-smoke-e2e-301). Expect `status: pending` and `creg status` → UNVERIFIED, not UNKNOWN.

## Sign-off

- **Operator:** ______________________ **Date:** __________
- **RPC used:** `https://ethereum-sepolia-rpc.publicnode.com` (automated run 2026-05-29)
- **First sync (s):** 20.4 **Restart sync (s):** 11.8
- **Results JSON:** `chain-registry/testnet/ops-201-logs/ops-201-results-20260529-225850.json`
- **Deltas from first run:** _______________________________________________

When complete, mark OPS-201 done in [NEXT_WORK.md](./NEXT_WORK.md) and update [PHASE2_CLOSEOUT.md](./PHASE2_CLOSEOUT.md).
