# Chain Registry — public documentation

> **Testnet fleet:** Public GCP hosting is **paused (cost)**; waitlist and Contribution Arena are **live**. Current matrix: [TESTNET_STATUS.md](./TESTNET_STATUS.md).

Curated entry point for **protocol, validators, publishers, and auditors**.  

At public scale CREG uses **hybrid analysis committees** (small mandatory core selected at admission + voluntary contributions). Committee data is public and queryable. See [CREG_SYSTEM_WORKFLOW.md](./CREG_SYSTEM_WORKFLOW.md#911-analysis-committee-selection-stake-weighting--caps) for details.
Hosting runbooks, Firebase deploy guides, budget models, and internal maintainer docs are kept in-repo for the core team but are **not** linked from the public README or waitlist.

**GitHub:** [chain-registry-blockchain-CREG-](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-) (canonical public repository).

Maintainers: see [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md) (not linked from waitlist or public README).

---

## Start here

| Document | Audience |
|----------|----------|
| [WHITEPAPER.md](./WHITEPAPER.md) | Developers, validators, auditors, ecosystem partners |
| [TESTNET_STATUS.md](./TESTNET_STATUS.md) | **What is live today** — fleet, waitlist, Arena |
| [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | Publishers, developers, validators |
| [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) | Testnet limits, verified semantics, alpha scope |
| [WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md) | Ed25519 (packages) vs Ethereum EOA (staking) |

---

## Protocol & architecture

| Document | Description |
|----------|-------------|
| [WHITEPAPER.md](./WHITEPAPER.md) | Public-alpha technical whitepaper |
| [DEEP_DIVE_ANALYSIS.md](../chain-registry/DEEP_DIVE_ANALYSIS.md) | Architecture, data flows, issue registry |
| [TESTNET_READINESS_REPORT.md](../chain-registry/TESTNET_READINESS_REPORT.md) | Evidence-based readiness snapshot |
| [chain-spec.sepolia.json](../chain-registry/testnet/chain-spec.sepolia.json) | Live testnet chain parameters |
| [contracts/README.md](../chain-registry/contracts/README.md) | Solidity contracts on Sepolia |
| [circuits/README.md](../circuits/README.md) | ZK Groth16 circuits (Circom) |
| [migrations/README.md](../chain-registry/migrations/README.md) | Database migration order (`db-sync`) |
| [DATABASE_SCHEMA.md](./DATABASE_SCHEMA.md) | PostgreSQL schema reference |

---

## Run a node (operators)

| Document | Description |
|----------|-------------|
| [testnet/QUICKSTART.md](../chain-registry/testnet/QUICKSTART.md) | Fastest path to a local node |
| [testnet/OPERATOR.md](../chain-registry/testnet/OPERATOR.md) | Sepolia external validators and GCP public-services overview |
| [testnet/README.md](../chain-registry/testnet/README.md) | Testnet directory index |
| [DOCKER.md](../chain-registry/DOCKER.md) | Docker Compose profiles |

---

## Build from source

```bash
cd chain-registry
cargo build --release -p cli
cargo test --workspace
cd contracts && forge test
```

Release binaries: [v0.4.4-testnet](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/v0.4.4-testnet) (Windows amd64). Linux/macOS archives remain [v0.4.3-testnet](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/v0.4.3-testnet) until Actions billing resets.

---

## Repository map (blockchain core)

| Path | Contents |
|------|----------|
| [`chain-registry/crates/`](../chain-registry/crates/) | Rust workspace — node, consensus, CLI, validators |
| [`chain-registry/contracts/`](../chain-registry/contracts/) | Staking, registry, governance, ZK verifier |
| [`circuits/`](../circuits/) | Circom sources and build scripts |
| [`chain-registry/schemas/`](../chain-registry/schemas/) | JSON schemas for packages and verdicts |
| [`chain-registry/rules/`](../chain-registry/rules/) | Supply-chain YARA rules |
| [`chain-registry/tests/`](../chain-registry/tests/) | Integration tests |

Web frontends (explorer, hub) and cloud deploy scripts live elsewhere in the monorepo and are **not** part of this public documentation set.

| Surface | URL | Doc |
|---------|-----|-----|
| Waitlist SPA | [waitlist.cregnet.dev](https://waitlist.cregnet.dev) | External [Creg-waitlist](https://github.com/samuel-1-avson/Creg-waitlist) repo |
| Contribution Arena | [contribute.cregnet.dev](https://contribute.cregnet.dev) | Separate educational/simulation product — not the protocol workstream |
