# Run a validator

Validators stake on Sepolia, register with the CREG staking contract, and operate nodes that participate in the coordinated testnet. This path is for operators comfortable with Docker, keys on secure hosts, and runbooks.

## Why validate?

- Exercise the full operator loop: fund → stake → register → sync.
- Help prove multi-validator behavior on the Sepolia lab.
- Surface operational docs gaps before wider fleet rollout.

## Prerequisites

- A dedicated **test** wallet with Sepolia ETH (gas) and test CREG/tokens per faucet docs.
- Hardware that can run the validator stack (see operator docs for CPU/RAM/disk).
- SSH or local access to a machine that will **hold keys** — never paste private keys into this hub.

## High-level steps

1. **Read the operator runbook** — `testnet/OPERATOR.md` in the repo covers compose layouts and security expectations.
2. **Fund the wallet** — [Faucet](https://faucet.testnet.cregnet.dev) for Sepolia ETH; follow testnet docs for CREG test tokens if required.
3. **Stake and register** — Use the explorer wallet tab or CLI scripts under `testnet/` (e.g. `register-validator-*-sepolia.ps1`).
4. **Run your node** — Local sandbox (`start-3node-sandbox.ps1`) or hybrid/GCP fleet per [GCP-VALIDATOR-FLEET.md](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/GCP-VALIDATOR-FLEET.md).
5. **Confirm on explorer** — [Validators list](https://explorer.testnet.cregnet.dev/validators) should show your registration when live.

## Security

- This hub **never** collects validator private keys.
- Enrollment intent capture and fleet queues are deferred to later phases.
- Use IAP/SSH and secrets managers for production-like deployments.

## Useful links

- [Explorer — Validators](https://explorer.testnet.cregnet.dev/validators)
- [Compare paths](/compare)
- [FAQ — Sepolia & faucet](/faq)
