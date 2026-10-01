# R7 source repairs and R29 faucet

This is a testnet source prerelease for the R7 chain and R29 faucet work. Publishing it makes the reviewed source and explorer artifact available; it does not deploy those changes to the running fleet or change chain state.

## Source and validation

- Source: [`main` at `73b005444fd77e95dcc3229116c757734d89b570`](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/commit/73b005444fd77e95dcc3229116c757734d89b570).
- [Rust CI run 36904100863](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/actions/runs/36904100863) passed formatting, Clippy with warnings denied, workspace tests, the Phase C production build, durable PBFT signing recovery, relayer validation, and the security-freeze guard.
- The source includes durable PBFT signing and prepared-block recovery, queue retirement, committed events, metric history, explorer data routing, relayer persistence, and typed Wasmtime fuel-exhaustion classification.
- The testnet consensus smoke job was skipped in this CI run. The private Contribution Arena workflow remains unverified because its submodule repository is not readable by the default Actions token.

## Explorer artifact

`creg-explorer-testnet-r29.tar.gz` was built from source commit `9d8bd71fdb33ad315641a69916a27f3f1a3204a4`. The subsequent commits to this source release do not change the explorer sources. Its SHA-256 is `29c74f98461436747c1f7ea9578c52dc2058edffab4d991736f2aeebae0d7963` (501,464 bytes). The release manifest records the source revision; `SHA256SUMS.txt` verifies the explorer archive and manifest.

## Operational evidence and scope

The attached public API and faucet snapshot was captured on 2026-09-30 at 05:36 UTC. It recorded R7 finalized height 20, four active validators, zero pending transactions, and a reconciled faucet ledger with 99,900 tCREG and 2,100 tCREG distributed. This is historical evidence, not a live health check at publication time.

This release performs no fleet rollout, chain reset, token mint or funding, contract migration, or public cutover. The separate CLI installer release remains [v0.4.9-testnet](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/v0.4.9-testnet). Refer to the release manifest for the complete scope and validation record.
