# R7 real multi-validator acceptance status

Captured: 2026-10-01 23:40:46 UTC
Chain: `creg-phase-c-testnet-2026-09-26-r7`
Shared finalized tip: **h20** — `730b01c1a67a4796a16f599169e4ebf77e01d2e8ce92d1e4fb3715a7c4354285`

## Result

All four validator endpoints and all four observer endpoints responded in the fresh capture. All eight reported the same chain, genesis, finalized height, and block hash. All PBFT round checks and recovery-queue checks were clear. This confirms shared finality at h20; it does **not** yet pass production acceptance.

The readiness gate is false for three specific reasons:

1. Every validator has only one configured Sepolia RPC provider host: `ethereum-sepolia-rpc.publicnode.com`. The production gate requires three independent provider hosts so a strict majority can tolerate one unavailable or inconsistent provider. No RPC URLs or credential values are recorded here.
2. Validator 4's L1 validator-set sync worker is stale: cursor block `11808024`, reported finalized source block `11808034`, last successful poll `2026-09-29T14:03:48Z`.
3. The eight runtime image IDs are not aligned; see the table below.

All four observers passed the capture checks. Validators 1–3 have fresh chain-authoritative L1 sync state, but fail the provider-count gate. Validator 4 fails both the provider-count and fresh-sync gates.

## Running images

| Node | Runtime image ID |
| --- | --- |
| V1 | `sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3` |
| V2 | `sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3` |
| V3 | `sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9` |
| V4 | `sha256:704cd7dfb89a048c146a3de502416c31a6f77c0e3f493a98f4fe3098c732046c` |
| O1 | `sha256:ef21b958b5e3e4f09e1c87290d3d9a50f23b5350d207e5af9982dd1cf5413708` |
| O2 | `sha256:2a553f1cb8b2ea6c63bf99649c57e0291698e3ac01da5de80863760afcc07a70` |
| O3 | `sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9` |
| O4 | `sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9` |

## V1 laptop recovery

Docker Desktop's Linux engine was stopped. Restarting Docker Desktop brought the existing V1 container back without rebuilding or replacing its image. V1 became healthy, reconnected to two peers, and reported the same h20 hash. No node data was removed, no chain reset was performed, and no package transaction was submitted.

## Work in progress

The local source now hardens L1 sync recovery: configured-endpoint quorum is required for heights and hashes, staking-event responses must match across a configured quorum, cursor changes are staged until the next cursor hash is quorum-confirmed, RPC errors are sanitized, and the worker retries after an unexpected exit. The readiness capture now checks L1 sync freshness, provider-host count, common finalized tip, and image alignment.

Eight pure quorum tests pass, the changed Rust files pass `rustfmt --check`, and the PowerShell capture script parses. The Windows Rust build could not finish because `librocksdb-sys` needs `libclang.dll`. The code changes are not in the running images and have not passed a Linux build yet.

## Required before the real package acceptance

1. Configure two additional independent Sepolia RPC providers alongside the current endpoint for V1–V4. Keep the API URLs in Secret Manager; supply secret **names only** for deployment configuration.
2. Build and validate the hardened node image, then recover V4's validator-set cursor and verify all four validator sync heartbeats are current.
3. Submit the previously approved benign test package only after all four validators pass the readiness gate. Require three validator votes, identical h21 hashes from all four validators, and h21 convergence on all four observers.
4. Preserve that acceptance evidence, then roll the verified image through the fleet with per-node health, chain-tip, recovery, PBFT, and artifact-digest checks.

## Evidence files

- [Fresh eight-node capture](fleet-readiness-final.json)
- [V1 restart follow-up capture](fleet-readiness-after-v1-restart.json)
- [Initial capture before V1 engine recovery](fleet-readiness-initial.json)
- [Capture using the stricter checks before V1 engine recovery](fleet-readiness-updated-gates.json)
