# Publish packages on CREG testnet

Publishing on **creg-testnet-1** lets you exercise the full registry flow: sign a package manifest, submit it on Sepolia, and confirm it appears in the public explorer.

## Why publish here?

- Test CLI and CI flows before mainnet.
- See how signatures and metadata land on-chain.
- Share reproducible packages with other testnet participants.

## Prerequisites

1. **Sepolia ETH** for gas — use the [testnet faucet](https://faucet.testnet.cregnet.dev) if needed.
2. **CREG CLI** installed (`creg` from this repository).
3. A **wallet** on Sepolia (chain id `11155111`).

## High-level flow

1. Build and sign your package locally with the CLI.
2. Submit to the Sepolia-backed registry API.
3. Open the **explorer** and search for your package hash or publisher address.

## Next steps

- Read [PUBLIC_TESTNET_QUICKSTART.md](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/PUBLIC_TESTNET_QUICKSTART.md) for commands and environment setup.
- Compare paths on [/compare](/compare) if you are unsure whether to publish or validate.
