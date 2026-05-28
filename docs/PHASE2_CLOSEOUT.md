# Phase 2 closeout checklist

**Date:** 2026-05-28

**Scope:** Sepolia Option A reuse — signed chain spec, live L1 contracts, one `creg-node` with chain-authoritative `validator_set_sync`.

## Verification

| Item | Status | Notes |
|------|--------|-------|
| REM-210 | Done | [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md) |
| SEC-203 | Done | `creg chain-spec validate testnet/chain-spec.sepolia.json` |
| SEC-101 / SEC-101b | Done | Hot-key runbook + startup fingerprint warnings |
| REM-103 / REM-103b | Done | Cursor sidecar; chunked `eth_getLogs`; cursor advances on empty deltas |
| Option A reuse script | Done | `testnet/run-sepolia-reuse.ps1` (+ spec server readiness poll) |
| L1 bytecode check | Done | `staking`, `registry`, `zk_verifier` on Sepolia |
| Spec signature at boot | Done | `CREG_SPEC_SIGNATURE_URL` + `CREG_SPEC_SIGNING_PUBKEY` |
| Sync to `synced` | Done | Public RPC; first walk ~9 min from `epoch_block_height: 0` |
| Cursor restart | Done | Restart → `synced` in ~10 s from saved cursor |
| REM-201 | Done | Governance API 501; explorer gated |

## Sepolia contract links (Option A reuse)

| Contract | Address | Etherscan |
|----------|---------|-----------|
| staking | `0xe58324Ce72718F802f3d6182e8eA06Cf91cc5d22` | [view](https://sepolia.etherscan.io/address/0xe58324Ce72718F802f3d6182e8eA06Cf91cc5d22) |
| registry | `0x3413EE0B398BE8696346ae294b28301E9AA2D16d` | [view](https://sepolia.etherscan.io/address/0x3413EE0B398BE8696346ae294b28301E9AA2D16d) |
| zk_verifier | `0x5aa70Af0e9c05A4e24485Ef72A7563976d919423` | [view](https://sepolia.etherscan.io/address/0x5aa70Af0e9c05A4e24485Ef72A7563976d919423) |

`genesis_hash` in spec: `0x64bba051625d4bfd7f3774b983983ed6ab87fdb7d0e486e1f689123a17d81ce3`

## Operator proof commands

```powershell
cd chain-registry
.\testnet\run-sepolia-reuse.ps1 -StartNode
Invoke-RestMethod http://localhost:8090/v1/health -TimeoutSec 30
cargo run --bin creg -p chain-registry-cli -- chain-spec validate testnet/chain-spec.sepolia.json
```

Expected health: `status=ok`, `validator_set_sync.state=synced`, `last_error` null.

## Deferred (post–Phase 2 ship)

| Item | Phase | Notes |
|------|-------|-------|
| SEC-105 | done | [WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md) |
| REM-211 | done | [OBSERVABILITY_SEPOLIA.md](./OBSERVABILITY_SEPOLIA.md) — Prometheus target UP; Grafana optional |
| REM-203 | Done | Merged [PR #6](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/6) |
| REM-211 | Done | Prometheus scrape + `/metrics` extensions |
| REM-202 | Deferred | Governance wiring (D3: keep disabled) |
| Second-operator runbook | **Open** | [SEPOLIA_SECOND_OPERATOR_CHECKLIST.md](./SEPOLIA_SECOND_OPERATOR_CHECKLIST.md) |
| Hot-key rotation drill | **Open** | [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) (`SEC-101`) |

## Phase 2 review checklist (plan)

- [x] REM-103 persistence verified with restart
- [x] Governance disabled (not stub) — D3 / REM-201
- [x] SEC-203, SEC-105, REM-203 on `main`
- [ ] Sepolia runbook executed by second engineer
- [ ] Hot-key rotation exercised once on testnet (optional procedural)

**Phase 3:** [PHASE3_KICKOFF.md](./PHASE3_KICKOFF.md) — decision **D4** recorded as SEC-303c (defer SEC-302).

## Merge history

- Phase 2 Sepolia ship: [PR #5](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/5) (`ce791d7`)
- REM-203 + metrics: [PR #6](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/pull/6) (`f6a4871`)
