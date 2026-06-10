# Publish on the testnet

Publishing lets you register packages on the CREG Sepolia lab and see them indexed by the network. This path is for developers who want to exercise the CLI, signing flow, and explorer visibility.

## Why publish?

- Validate your package signing and metadata workflow end-to-end.
- Appear on the [explorer](https://explorer.testnet.cregnet.dev) package list.
- Help the testnet team find publisher UX gaps before mainnet.

## Prerequisites

- A Sepolia-funded wallet (ETH for gas).
- The `creg` CLI installed — see [releases](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases) or `scripts/install-creg.ps1` / `install-creg.sh` in the repo.
- Testnet RPC access (public API or your own Sepolia endpoint).

## High-level steps

1. **Fund your wallet** — Claim Sepolia ETH from the [faucet](https://faucet.testnet.cregnet.dev).
2. **Install the CLI** — `creg --version` should print a build you trust.
3. **Configure Sepolia** — Point the CLI at the testnet API (`https://api.testnet.cregnet.dev`) and Sepolia RPC.
4. **Publish a package** — Follow [PUBLIC_TESTNET_QUICKSTART.md](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/PUBLIC_TESTNET_QUICKSTART.md) for the exact commands.
5. **Verify on explorer** — Open your package page and confirm status.

## Useful links

- [Explorer — Packages](https://explorer.testnet.cregnet.dev/packages)
- [Testnet API health](https://api.testnet.cregnet.dev/v1/health)
- [Compare paths](/compare) — Publisher vs validator vs observer

Quest checklists and progress tracking arrive in Phase 2 (SIWE sign-in).
