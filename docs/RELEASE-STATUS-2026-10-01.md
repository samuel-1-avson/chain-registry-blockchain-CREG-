# R7 source and R29 testnet prerelease report

Updated: 2026-10-01 UTC

## Published source

The source release is published from `main` at commit `73b005444fd77e95dcc3229116c757734d89b570`.

The release contains the prior R7 source repairs for durable PBFT signing and prepared-block recovery, queue retirement, committed events, metric history, explorer data routing, relayer persistence, plus typed Wasmtime fuel-exhaustion classification. The recent source-release history includes `697d7ba`, `08850c4`, `63bb660`, `09f9a12`, `a1d7354`, `b80a4b0`, and `73b0054`.

## Releases

- [R7 source repairs and R29 faucet prerelease](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/testnet-r29): published as a prerelease, targeted at `73b005444fd77e95dcc3229116c757734d89b570`.
- The attached explorer archive was built from `9d8bd71fdb33ad315641a69916a27f3f1a3204a4`; its SHA-256 is `29c74f98461436747c1f7ea9578c52dc2058edffab4d991736f2aeebae0d7963`.
- The release includes `testnet-r29-release-manifest.json` and `SHA256SUMS.txt`. The manifest records source, CI, artifact, operational snapshot, and scope.
- [CLI v0.4.9-testnet](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/v0.4.9-testnet) remains a separate installer release.

## Verification

| Check | Result |
| --- | --- |
| Rust CI | [Passed](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/actions/runs/36904100863): formatting, Clippy with warnings denied, full workspace tests, Phase C production build, durable PBFT signing recovery, relayer validation, and security-freeze guard |
| WASM sandbox on this Windows host | 5/5 platform-applicable unit tests passed. The Unix-only fuel exhaustion test ran in the passing Linux workspace CI. |
| Historical public explorer and node snapshot | Captured 2026-09-30 05:36 UTC: R7 finalized height 20, 21 blocks including genesis, four active validators, and zero pending transactions. This is a dated snapshot, not a current health check. See [prepublication checks](testnet-heal-evidence/2026-09-30-source-release/prepublication-public-checks.json). |
| Historical faucet snapshot | Same capture reported a reconciled ledger, 99,900 tCREG balance, 2,100 tCREG distributed, and zero unresolved claims. It is not a current health check. |
| Private Contribution Arena workflow | Not verified: its private submodule is not accessible to the default Actions token. |
| Testnet consensus smoke job | Skipped by the Rust CI workflow. |

The full release manifest is [available in the repository](releases/testnet-r29-release-manifest.json) and as a GitHub release asset.

## Scope and remaining work

Publishing the source prerelease did not roll out services to the validator or observer fleet, reset the chain, mint or transfer tokens, migrate contracts, or perform public cutover. Live deployment and end-to-end acceptance remain separate work. Do not treat the historical explorer or faucet snapshot as confirmation of current service health.

The private submodule workflow still needs appropriately scoped repository access. Economic issuance toward the agreed 42,000,000 tCREG testnet target, reward and pinning payout activation, and remaining real-fleet acceptance work remain governed by the [tokenomics implementation plan](CREG-TOKENOMICS-IMPLEMENTATION-PLAN.md).
