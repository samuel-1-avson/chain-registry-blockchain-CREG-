# Sepolia Option A — second operator checklist

Use this when a second person repeats the reuse path to close the Phase 2 ops gap ([PHASE2_CLOSEOUT.md](./PHASE2_CLOSEOUT.md)).

**Prerequisites:** Docker, Rust toolchain, Sepolia RPC access, spec signing pubkey (from operator who published the spec).

## Checklist

| Step | Action | Pass? | Notes / deltas |
|------|--------|-------|----------------|
| 1 | Clone repo, checkout branch with Phase 2 ship | ☐ | Branch: _______________ |
| 2 | `cd chain-registry` | ☐ | |
| 3 | `cargo build --bin creg-node -p chain-registry-node` | ☐ | |
| 4 | Set `CREG_SPEC_SIGNING_PUBKEY` (from spec publisher) | ☐ | |
| 5 | `.\testnet\run-sepolia-reuse.ps1` (no `-StartNode`) | ☐ | Spec server :8888 healthy |
| 6 | `.\testnet\run-sepolia-reuse.ps1 -StartNode` | ☐ | API :8090 |
| 7 | `Invoke-RestMethod http://localhost:8090/v1/health` | ☐ | `validator_set_sync.state` = `synced` |
| 8 | `cargo run --bin creg -p chain-registry-cli -- chain-spec validate testnet/chain-spec.sepolia.json` | ☐ | exit 0 |
| 9 | Stop node, restart with same `sepolia-node-data` | ☐ | Resync in seconds, not minutes |
| 10 | Record operator name, date, RPC URL used | ☐ | |

## Sign-off

- **Operator:** ______________________ **Date:** __________
- **Deltas from first run:** _______________________________________________

When complete, add a line to `PHASE2_CLOSEOUT.md` under merge notes or link this file from Step 6 in `PHASE2_SEPOLIA_KICKOFF.md`.
