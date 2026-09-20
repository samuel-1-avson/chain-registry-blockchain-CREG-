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
cargo build --locked --release --package chain-registry-cli --bin creg \
  --package chain-registry-node --bin creg-node
export CREG_NODE_URL=https://api.testnet.cregnet.dev
./target/release/creg doctor
```

Full publisher and validator flows: [testnet/QUICKSTART.md](testnet/QUICKSTART.md) and [testnet/OPERATOR.md](testnet/OPERATOR.md).

### Updating an installed CLI

The installed `creg` binary can check the configured GitHub release stream and
stage a verified platform update without downloading a full source tree:

```bash
creg update --check       # inspect the newest release; change nothing
creg update               # show the candidate and require confirmation
creg update --yes         # verify SHA-256, stage, and install the update
creg update --version v0.4.1-testnet --yes  # explicit release/rollback rehearsal
```

The updater selects the current OS/CPU archive, requires a release digest (the
GitHub asset digest or a matching `SHA256SUMS` entry), stages the `creg` binary,
and replaces it only after verification. On Windows replacement is deferred to
a short-lived helper after the running process exits. Set `CREG_GITHUB_REPO`
to use a controlled fork or mirror. Updates never read or handle validator
private keys. The command updates the `creg` CLI; a running `creg-node` should
be upgraded through the operator's service/deployment restart so its data
directory and validator process are never replaced underneath it.

---

## Documentation

Core docs are kept in root, docs/, testnet/, and observability/ (see docs/archive/README.md for what was archived as non-core).

Key references:
- [DEEP_DIVE_ANALYSIS.md](DEEP_DIVE_ANALYSIS.md) — architecture and issue registry
- [../docs/PROTOCOL_PROGRESS.md](../docs/PROTOCOL_PROGRESS.md) — current maintainer briefing ([WHAT_TO_DO_NEXT.md](../docs/archive/WHAT_TO_DO_NEXT.md) archived 2026-09-19)
- [testnet/QUICKSTART.md](testnet/QUICKSTART.md)
- [testnet/OPERATOR.md](testnet/OPERATOR.md)
- [testnet/gcp/](testnet/gcp/) — GCP scripts (no README yet)
- [DOCKER.md](DOCKER.md)
- [contracts/README.md](contracts/README.md)

---

## Contributing

```bash
cargo test --workspace
cd contracts && forge test
```

Reference issue IDs from [DEEP_DIVE_ANALYSIS.md](DEEP_DIVE_ANALYSIS.md) in pull requests.
