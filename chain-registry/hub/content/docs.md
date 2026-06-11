# Documentation

Operator guides and developer quickstarts for the CREG Sepolia testnet (`creg-testnet-1`).

## Start here

| Guide | Description |
|-------|-------------|
| [Publish journey](/publish) | Stake, IPFS, and ship your first package |
| [Validate journey](/validate) | Validator stake, keys, and fleet operations |
| [Compare paths](/compare) | Publisher vs validator vs observer |
| [FAQ](/faq) | Faucet limits, wallets, and support |

## Repository docs (full detail)

| Topic | Link |
|-------|------|
| Public testnet quickstart | [PUBLIC_TESTNET_QUICKSTART.md](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/PUBLIC_TESTNET_QUICKSTART.md) |
| Phase scope & limits | [TESTNET_PHASE_SCOPE.md](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/TESTNET_PHASE_SCOPE.md) |
| Sepolia operator runbook | [TESTNET_SEPOLIA_RUNBOOK.md](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/TESTNET_SEPOLIA_RUNBOOK.md) |
| Wallet & key types | [WALLET_KEY_DERIVATION.md](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/WALLET_KEY_DERIVATION.md) |
| Readiness report | [TESTNET_READINESS_REPORT.md](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/chain-registry/TESTNET_READINESS_REPORT.md) |

## Live services

| Service | URL |
|---------|-----|
| Join hub | `https://testnet.cregnet.dev` |
| Explorer | [explorer.testnet.cregnet.dev](https://explorer.testnet.cregnet.dev) |
| Faucet | [faucet.testnet.cregnet.dev](https://faucet.testnet.cregnet.dev) |
| Chain spec | [spec.testnet.cregnet.dev](https://spec.testnet.cregnet.dev/chain-spec.json) |
| Node API reference | [/api-reference](/api-reference) on this site → [Swagger UI](https://api.testnet.cregnet.dev/api-docs/) |

## CLI essentials

```bash
export CREG_NODE_URL=https://api.testnet.cregnet.dev
creg keygen publisher --out ~/.creg/publisher.key
creg testnet drip --address 0xYourAddress
```

Build the CLI from the repository root: `cargo build --release -p cli`.
