# Chain Registry — protocol workspace

Rust validator network, `creg` CLI, Solidity contracts, ZK circuits, and testnet configuration for **CREG** (`creg-testnet-1` on Sepolia).

This directory is the **public blockchain surface** of the repository — comparable to how Bitcoin Core or go-ethereum expose node and consensus code, not product web apps or hosting runbooks.

**Browse on GitHub:** [github.com/.../tree/main/chain-registry](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/tree/main/chain-registry)

---

## What is here

| Area | Path |
|------|------|
| Node & consensus | [`crates/`](crates/) — `node`, `consensus`, `validator`, `cli`, `db-sync`, … |
| Smart contracts | [`contracts/`](contracts/) — Sepolia deployment scripts and tests |
| ZK circuits | [`../circuits/`](../circuits/) — Groth16 package-validation circuits |
| Chain spec | [`testnet/chain-spec.sepolia.json`](testnet/chain-spec.sepolia.json) |
| Integration tests | [`tests/`](tests/) |
| Schemas & rules | [`schemas/`](schemas/), [`rules/`](rules/) |

---

## Quick start

```bash
cargo build --release -p cli
export CREG_NODE_URL=https://api.testnet.cregnet.dev
./target/release/creg doctor
```

Full publisher and validator flows: [docs/PUBLIC_TESTNET_QUICKSTART.md](../docs/PUBLIC_TESTNET_QUICKSTART.md).

---

## Documentation

**[docs/PUBLIC.md](../docs/PUBLIC.md)** — curated public docs (protocol, operators, contracts).

Architecture deep dive: [DEEP_DIVE_ANALYSIS.md](DEEP_DIVE_ANALYSIS.md).

---

## Contributing

```bash
cargo test --workspace
cd contracts && forge test
```

Reference issue IDs from [DEEP_DIVE_ANALYSIS.md](DEEP_DIVE_ANALYSIS.md) in pull requests.
