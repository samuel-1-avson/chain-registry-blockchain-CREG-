# Run a validator on CREG testnet

Validators stake on Sepolia, register with the registry, and operate **creg** nodes that follow **creg-testnet-1**. This path is for operators comfortable with keys, RPC, and basic server ops.

## Why validate?

- Exercise real network security and governance flows.
- Learn stake, registration, and node lifecycle before production.
- Support package publishers by running consensus infrastructure.

## Prerequisites

1. **Sepolia ETH** for stake transactions and gas.
2. A **dedicated validator key** — never paste private keys into this site; use your local operator environment only.
3. Hardware that can run the validator stack (see operator docs for sizing).

## High-level flow

1. Fund your operator wallet on Sepolia.
2. Stake and register via CLI or operator scripts.
3. Deploy and monitor your node; confirm status on the **explorer**.

## Security

The join portal **never** collects validator private keys. Enrollment wizards and quest progress come in a later phase; for now follow the operator runbooks in the repository.

## Next steps

- Operator and fleet docs in `docs/` (GCP validator fleet, hybrid local validators).
- Use [/compare](/compare) to contrast validator vs publisher responsibilities.
