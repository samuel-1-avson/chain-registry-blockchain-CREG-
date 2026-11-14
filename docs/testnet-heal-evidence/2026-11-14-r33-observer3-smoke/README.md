# r33 node image: observer-3 smoke test (2026-11-14)

**Result: passed.** Observer-3 runs the r33 node image with its RPC configuration unchanged. No validator, other observer, DNS entry or RPC setting was changed.

## Image

| | |
|---|---|
| Source | `eea0e1caa5d4a13147702657a9d0330117534727` (merge of #59; includes #57, #58, #59) |
| Cloud Build | `d31790e0-d438-4339-b931-6cb0ddf14cce`, project `gen-lang-client-0022105784`, `us-central1`. SUCCESS, 17:08–18:15 UTC, Rust 1.93.0, `node-runtime` target, `phase-c-real-fleet` |
| Tag | `us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node:phase-c-rpc-r33-rust193-eea0e1caa5d4` |
| Registry digest | `sha256:d803682a69c2705c7679c0f7a717aa45b2dd4286ddd902256401ca59a41611e7` |
| Config digest (image ID on the VM) | `sha256:dbc881f366152da3edd5988eb5f7a759def3707e58a3b1c3c83642879016eac7`, matched against the registry manifest |
| Source manifest SHA-256 | `74a9da63b74ecfdd94bd7d38e8f4eafec20d8daaedd9fde0ee01c02778663843` (236 files) |
| `--version` | `creg-node 0.1.0` |
| `/app/creg-node` SHA-256 | `f7476890689b1087b8594646fabc25ccde7acef46e86f1e23c725b8a0d8838e1`: recorded by Cloud Build, re-checked on observer-3 before the swap, and checked again in the running container |

Observer-3 has no Artifact Registry read access. The image was pulled on the maintainer laptop, saved, copied over IAP (`a344d095…` on both ends) and loaded. No credential was placed on the VM.

## Swap

Host `creg-observer-3` (`fiffi-500411`, `us-west1-b`), container `creg-phase-c-r7-o3`, swapped at 19:02:36 UTC.

What was kept the same:
- The env file, `/var/lib/creg/phase-c-r7/node.env`. Before the swap it matched the running container exactly; the only difference was `CREG_DATA_DIR`, which the image also sets to `/data`.
- Ports `8080` and `9000`.
- The data bind mount, read-write.
- The scanner mount `scanner-r14-ansi-regex-live`, read-only.
- Restart policy `unless-stopped`.

What was preserved for rollback:
- The old container, `creg-phase-c-r7-o3-r22-before-r33`, stopped (image `8731cc85c73e…`).
- A data copy, `/var/lib/creg/phase-c-r7/data-before-r33-20261114T190236Z` (14 MB), taken while the node was stopped and checked with `diff -r`.

## Readiness checks (all passed)

| Check | Result |
|---|---|
| New image running, binary matches the build | yes |
| Signed chain spec verified, genesis `0f2a3b10…` unchanged | yes |
| Chain not behind: tip and finalized height 20, tip hash `730b01c1…` unchanged | yes |
| Packages kept (8) | yes |
| 4 of 4 validators | yes |
| Observer role, peers > 0 (2) | yes |
| Scanner profile `7e1d95f9…` unchanged and ready | yes |
| Durable work, pending and forced-inclusion queues clear; PBFT rounds clear | yes |
| New `creg_chain_tip_age_seconds` metric present | yes |
| P2P identity unchanged | yes |

After one minute: healthy, 0 restarts, 0 `ERROR` log lines. Startup logged `L1 chain id: 11155111 verified by 1/1 configured RPC endpoints`.

Full evidence JSON on the host: `/var/lib/creg/phase-c-r7/evidence/r33-o3-smoke-20261114T190236Z.json`.

## Unchanged, as expected

- **Validator-set sync is still degraded.** The cursor is at Sepolia block 11,850,628 against a finalized source of 12,135,055. The only RPC is PublicNode, which does not serve the historical Staking logs. The new image now reports this as "did not reach staking-log quorum" and warns that only one endpoint is configured.
- **`creg_chain_tip_age_seconds` = 4,096,063** (about 47 days). The CREG chain has produced no block since late September. With no pending work, `CregBlockProductionStalled` stays quiet and `CregChainIdle` (info) applies.
- **A warning repeats every 5 seconds:** "PBFT signing is blocked because its durable safety journal is not initialized". This is existing behaviour on observers: the old container logged it 136,649 times since 2026-09-28. It is a candidate for lowering to debug level on non-validators.

## Rollback

On the host, run the script with the backup path above:

```bash
bash /tmp/o3-rollback-r33.unix.sh /var/lib/creg/phase-c-r7/data-before-r33-20261114T190236Z
```

It does the following:
- stops the r33 container and renames it to `creg-phase-c-r7-o3-r33-failed-<ts>`;
- moves the current data aside;
- restores the backup;
- renames the preserved r22 container back to `creg-phase-c-r7-o3` and starts it.

## Next (from the 2026-10-02 checkpoint)

1. Two full-history providers are configured as a new version of `creg-sepolia-rpc-profile` in Secret Manager.
2. Geth reaches the Sepolia head.
3. All three providers agree on chain ID and on the block hash at a common finalized checkpoint.
4. All three return the same normalized Staking log set from block 11,781,426 (`0xb3c532`).
5. Only then, set `CREG_ETH_RPC` / `CREG_ETH_RPC_FALLBACKS` on observer-3 and confirm validator-set sync reaches `synced`.
6. Then roll out to the other observers, and to validators one at a time, preserving their signing journals.
