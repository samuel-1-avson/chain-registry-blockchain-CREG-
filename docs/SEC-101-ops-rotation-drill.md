# SEC-101-ops — hot-key rotation drill (testnet)

One-time operator exercise per [NEXT_WORK.md](./NEXT_WORK.md) P0 item 3. Full policy: [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md).

## Scope

Rotate **one** testnet hot key without taking down the Sepolia reuse path:

| Key env var | Service | Drill priority |
|-------------|---------|----------------|
| `CREG_BRIDGE_KEY` | Node bridge worker | Preferred if bridge is enabled |
| `FAUCET_PRIVATE_KEY` | Faucet | If faucet container runs |
| `RELAYER_PRIVATE_KEY` | Relayer | If relayer runs |

Do **not** rotate `CREG_SPEC_SIGNING_PUBKEY` or republish `chain-spec.sepolia.json` in this drill.

## Preconditions

- [x] Sepolia node healthy: `validator_set_sync.state` = `synced` ([OPS-201](./SEPOLIA_SECOND_OPERATOR_CHECKLIST.md) — automated verify 2026-05-29)
- [x] `CREG_TESTNET=true` on affected services (`.env.sepolia` 2026-05-30)
- [ ] New key generated (`cast wallet new` or KMS)
- [ ] New address funded on Sepolia if it must send transactions

## Drill steps

1. **Record** current startup log fingerprint for the chosen key (non-testnet nodes log `hot key loaded` with fingerprint only).
2. **Stop** the service using the key (node bridge worker, faucet, or relayer).
3. **Update** secret in vault / local `.env` (never commit).
4. **Revoke** old on-chain roles if applicable (staking operator, etc.).
5. **Start** service with `CREG_TESTNET=true`.
6. **Verify**
   - Node health still `ok` if node was running: `Invoke-RestMethod http://localhost:8090/v1/health`
   - Faucet/relayer smoke (if used): one successful test transaction or health endpoint
   - Logs show new fingerprint when `CREG_TESTNET` is not set (staging check only)
7. **Record** date, operator, key type rotated, and any incident in your security log.

## Sign-off

| Field | Value |
|-------|-------|
| Operator | (fill name) |
| Date | 2026-05-30 |
| Key rotated | `CREG_BRIDGE_KEY` |
| Before fingerprint | `(placeholder)` — `sec-101-drill-before-20260530-023325.log` |
| After fingerprint | `0x2b456b84...332dc478` — `sec-101-drill-after-20260530-023813.log` |
| Sepolia health after drill | pass — OPS-201 verify 2026-05-30 (12.2s / 11.9s sync) |
| Notes | `set-bridge-key-env.ps1`; observer node; new PowerShell for verify |

When complete, mark SEC-101-ops done in [NEXT_WORK.md](./NEXT_WORK.md).

## Automation

```powershell
cd chain-registry

# Before (loads testnet/.env.sepolia automatically)
.\testnet\run-sec-101-drill.ps1 -Label before

# Generate new secp256k1 key (Foundry)
cast wallet new
# Put the printed private key in .env.sepolia as CREG_BRIDGE_KEY=0x<64 hex chars>
# Ensure: CREG_TESTNET=true

# Stop node on :8090 if running, then restart Sepolia path:
.\testnet\run-sepolia-reuse.ps1 -StartNode
# Or background via run-ops-201-verify.ps1 -SkipPublish

.\testnet\run-sec-101-drill.ps1 -Label after
# Fingerprints for CREG_BRIDGE_KEY must differ before vs after
```

Logs: `testnet/ops-201-logs/sec-101-drill-*.log`
