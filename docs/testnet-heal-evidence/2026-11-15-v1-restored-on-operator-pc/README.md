# Validator-1 restored on the operator PC with its original key (2026-11-15)

**Result:** validator-1 runs again on the operator PC, for the first time since 2026-10-02. All four R7 validators are up. Option A of [the recovery plan](../../VALIDATOR-1-RECOVERY-PLAN-2026-11-15.md) was followed:
- the original key, from Secret Manager;
- its own data and signing journal;
- no new key, no Sepolia transaction, no Governance vote, and no change to any other node.

**Not yet shown:** that validator-1 signs. The chain has had no PBFT round since late September, so this shows only on the next block.

## Fleet now

| Slot | Runs on | State |
|---|---|---|
| v1 (validator-1) | Operator PC, container `creg-phase-c-r7-v1`, API `18081`, P2P `9002` | **Running** (restored 11:27:58 UTC) |
| v2 (validator-2) | Operator PC, container `creg-phase-c-r7-v2`, API `18080`, P2P `9001` | Running (moved from GCP the same day) |
| v3 (validator-3) | GCP `creg-validator-3`, `europe-west1-b` | Running |
| v4 (validator-4) | GCP `creg-validator-4`, `asia-southeast1-b` | Running |

- **While the PC is on**, all 4 validators sign and the chain survives one cloud outage.
- **While the PC is off or asleep**, 2 of 4 sign and the chain cannot finalize. That was already the case before the restore, so the restore only adds margin.

## Steps and checks

1. **The key was confirmed without being shown.** Secret `creg-fresh-testnet-validator-1-ed25519` (version 1, project `gen-lang-client-0022105784`) was read on the PC. Its public key was derived with Node's built-in Ed25519 support: `cc4157c4443c9fb011df2b650f497e3eadfc1ddda6dc7f4b6732325383108ffc`, R7 validator-1's registered key. Only the public key was printed ([`scripts/derive-ed25519-pub.js`](./scripts/derive-ed25519-pub.js)).
2. **Nothing else holds that key.** On every cloud host the public key of each container's `CREG_VALIDATOR_KEY` was derived, running or stopped ([`scripts/fleet-keys.sh`](./scripts/fleet-keys.sh)):
   - `creg-validator-3`: validator-3's key only, in all 17 containers.
   - `creg-validator-4`: validator-4's key only, in all 26 containers.
   - The four observers: no validator key at all.
   - The PC: validator-2's key only.
3. **The data was backed up.** `C:\Users\samue\.creg-phase-c\r7\v1\data` was copied to `C:\Users\samue\.creg-phase-c\r7\v1-data-before-restore-20261115T112746Z` (152 files, 13,973,541 bytes, counts and sizes equal).
4. **The env file was rebuilt** at `C:\Users\samue\.creg-phase-c\r7\v1-private\node.env`, readable only by the operator account and SYSTEM. It is validator-2's file with exactly two lines changed: `CREG_NODE_ID=validator-1` and `CREG_VALIDATOR_KEY` from the secret. The key was checked again before writing.
5. **The container started** with the same image as v2 (`2b472a08…`, binary `c7ef1d3e…`), the same privileged sandbox and Docker socket, v1's own data folder and the shared read-only scanner bundle ([`scripts/v1-restore.ps1`](./scripts/v1-restore.ps1)).

**Readiness** passed 24 seconds after start:
- Docker reports it healthy, the image matches, and the node reports `validator-1` as validator.
- The signed chain spec is verified, genesis `0f2a3b10…`, tip and finalized height 20 with hash `730b01c1…`.
- 4 of 4 validators are registered and peers are connected.
- The scanner profile `7e1d95f9…` is ready, the queues and PBFT rounds are clear, and the sandbox is isolated.
- The P2P identity is unchanged (peer `12D3KooWMpALWEjVhPTJ8Hjxk3ZVYeYXhX8LkvwrjCjjzTSmFvXA`).
- **The signing journal opened with no identity error.** It is bound to `{genesis, validator-1, cc4157c4…}`, so this proves the restored key is the one it has always protected.
- 0 ERROR lines.

**Seen by peers:**
- v3 logged 8 connections from validator-1 and o4 logged 16 within minutes; both now have 6 peers.
- v4 has no direct link to either PC validator. It reaches them through v3 and o4, the same as before the move.

## Rollback

```bash
docker update --restart=no creg-phase-c-r7-v1
```

```bash
docker stop creg-phase-c-r7-v1
```

Stopping v1 returns the fleet to 3 of 4. Do not start validator-1 anywhere else while this container exists, because that is a double-sign. If the data folder ever has to be restored, use the newest copy, never an older one.

## Follow-ups

- Find out how `C:\Users\samue\CREG-backups` and the old v1 container were lost. If the folder may have been copied elsewhere, rotate validator-1's key (plan Option B).
- validator-2's key is listed in Secret Manager too (`creg-fresh-testnet-validator-2-ed25519`). Once its public key is confirmed as `779c9522…`, the GCP snapshot `creg-validator-1-v2-moved-20261115` is only a second backup.
- Both PC validators fail together. Moving one to another always-on machine would let the chain survive the PC being off.
