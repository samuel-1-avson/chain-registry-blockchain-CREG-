# Validator-2 moved from GCP to the operator PC (2026-11-15)

> **Update, later the same day:** validator-1 was restored on the same PC with its original key, so all four validators run again. See [the v1 restore record](../2026-11-15-v1-restored-on-operator-pc/README.md). The 3-of-4 warnings below describe the state between the two changes.

**Result:** validator-2 now runs on the operator PC in Docker Desktop. Its GCP VM `creg-validator-1` is deleted. Its identity, signing key and signing journal came across unchanged, so nothing changed on Sepolia and the R7 validator set is the same.

**Not yet shown:** that validator-2 signs from the PC. The chain has had no PBFT round since late September, so this shows only on the next block.

## Why

The operator wants validators on their own machine rather than in the cloud. Validator-2 was chosen; validator-3 and validator-4 stay on GCP.

## Fleet before and after

| Slot | Before | After |
|---|---|---|
| v1 (validator-1) | Operator PC, **down since 2026-10-02**, signing key file lost | Unchanged: still down |
| v2 (validator-2) | GCP VM `creg-validator-1`, `us-east1-b` | **Operator PC**, container `creg-phase-c-r7-v2` |
| v3 (validator-3) | GCP `creg-validator-3`, `europe-west1-b` | Unchanged |
| v4 (validator-4) | GCP `creg-validator-4`, `asia-southeast1-b` | Unchanged |

**Three of four validators are signing, which is exactly the PBFT quorum.** The chain finalizes only while v2, v3 and v4 are all up, and v2 now depends on the PC being on and awake.

## What was checked before the move

- **Nothing else depended on the VM.** The chain-spec host `spec.testnet.cregnet.dev` resolves to `creg-testnet-vm`, the VM's IP was ephemeral with no DNS record, and no monitoring config referenced it.
- **No other node holds validator-2's key, and the key is the registered one.** The three cloud validators had three different key fingerprints. The key in the VM's `node.env` matched the running container, and its derived public key is R7 validator-2's registered `779c9522…`.
- **The VM's leftover container was not useful.** `creg-validator-validator-1` (image `genesis2`, chain `creg-testnet-1`) was crash-looping on a spec-key mismatch, 1,735 restarts. Its key derives to `b3068f39…`, which is not an R7 validator, so it could not restore validator-1.
- **The PC has the same connectivity as the VM.** From the PC, P2P port 9000 on v3, v4 and o4 is reachable. HTTP port 8080 is closed from both the PC and the VM, so nothing changed there.

## How it was moved

1. The PC pulled the exact image the VM ran, `2b472a08…`; the binary `c7ef1d3e…` matched.
2. On the VM, validator-2 was checked at tip 20 with no PBFT round open. Then both containers were set to `restart=no` and stopped (10:57:38 UTC).
3. `data` (chain store, signing journal, P2P identity), `node.env` and the scanner bundle were packed. They were copied over IAP, checked by SHA-256 on both ends, and unpacked on the PC. The package was then shredded on the VM.
4. The P2P identity and the validator key were confirmed unchanged by fingerprint, without printing either.
5. The container started on the PC at 10:58:43 UTC with the same settings as on the VM ([`scripts/v2-move.ps1`](./scripts/v2-move.ps1)):
   - the same image, `node.env`, privileged sandbox and Docker socket;
   - ports `18080` (API) and `9001` (P2P);
   - the data mount read-write and the scanner mount read-only.

   The VM step ran from [`scripts/v2-stop-pack.sh`](./scripts/v2-stop-pack.sh). It was built so that two copies could never run at once:
   - if anything had failed before the PC start, it would have restarted validator-2 on the VM;
   - if anything had failed after the start, both copies would have stayed stopped.

## Checks on the PC (all passed, 21 seconds after start)

- Docker reports the container healthy, and the image matches.
- The node reports `validator-2` with `is_validator` true.
- The signed chain spec is verified, genesis `0f2a3b10…`, tip and finalized height 20 with hash `730b01c1…`.
- 4 of 4 validators are registered, and peers are connected.
- The scanner profile `7e1d95f9…` is ready, the queues are clear, the PBFT rounds are clear and the sandbox is isolated.
- The P2P identity is unchanged, and there are 0 ERROR log lines.

**Peers see it:**
- v3 and o4 log connections from the PC's peer `12D3KooWChgc…`, and the PC is connected to both.
- v4 has no direct link and reaches validator-2 through v3 and o4, the same seeds validator-2 used from GCP.

The only warnings are the known validator-set-sync ones (single RPC endpoint, pruned history), the same as on the VM.

Record: [`v2-move.json`](./v2-move.json).

## Deleted (confirmed by the operator)

- VM `creg-validator-1` (`gen-lang-client-0022105784`, `us-east1-b`, e2-medium), deleted at 11:13:26 UTC.
- Its 30 GB boot disk (auto-delete), deleted with the VM.
- Its ephemeral IP `34.23.56.23`, released.

**Kept as a backup:** snapshot `creg-validator-1-v2-moved-20261115` (READY, about 10 GB stored, multi-region `us`). It holds validator-2's key and its pre-move data. It costs roughly $1 a month. Delete it only once the key is backed up somewhere else. The snapshot and deletion ran from [`scripts/v2-delete-vm.ps1`](./scripts/v2-delete-vm.ps1). To delete the snapshot later:

```bash
gcloud compute snapshots delete creg-validator-1-v2-moved-20261115 --project=gen-lang-client-0022105784
```

## On the PC

| | |
|---|---|
| Container | `creg-phase-c-r7-v2`, restart `unless-stopped` |
| API | `http://127.0.0.1:18080` |
| Data | `C:\Users\samue\.creg-phase-c\r7\v2\data` (outside OneDrive) |
| Key file | `C:\Users\samue\.creg-phase-c\r7\v2-private\node.env`, readable only by the operator account and SYSTEM |
| Scanner bundle | `C:\Users\samue\.creg-phase-c\r7\v2\scanner-r14-ansi-regex-live` (read-only mount) |

Operating notes:
- **Keep the PC on and awake** while the chain should make blocks. While validator-1 is down, the chain cannot finalize when this PC sleeps.
- **Docker Desktop must start when you sign in.** The container then comes back by itself.
- **Back up the key file.** Validator-1 was lost when `C:\Users\samue\CREG-backups` disappeared. Apart from the snapshot, the PC holds the only copy of validator-2's key.
- **Never start a second copy** of validator-2, for example from the snapshot, while this one runs. That is a double-sign (ADR-016).

## Rollback (to GCP)

1. Stop and disable the PC container:
   ```bash
   docker update --restart=no creg-phase-c-r7-v2
   ```
   ```bash
   docker stop creg-phase-c-r7-v2
   ```
2. Create a VM from the snapshot, or copy `C:\Users\samue\.creg-phase-c\r7\v2` and `v2-private\node.env` to a new host.
3. Start the container there with the same image and settings. Use the PC's data: it is newer than the snapshot once the PC has signed anything.
