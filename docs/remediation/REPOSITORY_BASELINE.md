# Repository baseline — 2026-07-14

The repository root contains the Rust workspace under `chain-registry/`,
Solidity contracts under `chain-registry/contracts/`, root GitHub workflows,
documentation under `docs/`, and two separately scoped web/submodule trees.

Current core binaries/services include the node, CLI, validator, consensus,
resolver, IPFS pinner, DB sync, faucet, relayer, sandbox, ZK, ML and related
experimental crates. The public API includes REST/gRPC node surfaces and the
Hub API. Docker Compose files, testnet scripts, SQL migrations, GCP materials,
and root/inner CI workflow files provide deployment and operations scaffolding.

Canonical source of truth is **not yet implemented**: the local RocksDB chain,
Solidity registry/checkpoint records, PostgreSQL-derived data, IPFS objects,
and API caches must be treated as potentially conflicting legacy/derived
stores. ADR-002/003 select a future witnessed append-only log as canonical.

Active dangerous paths found in source/configuration: legacy custom consensus,
legacy reward/token/cross-chain/private feature surfaces, incomplete
freshness, incomplete durable queue, and undeployed contract migrations. ZK
qualification is now disabled in all repository-shipped chain specifications
and reverts in the current Registry source; that does not alter a remote signed
specification or deployed bytecode.

Topology evidence in the Sepolia specification is one validator and no
bootnodes; this is a coordinated research alpha, not BFT or decentralized.
The repository contains existing uncommitted Rust, Hub API, and testnet edits
listed by `git status` on 2026-07-14. They were preserved and not represented
as independently verified fixes in this baseline.

Baseline command results are recorded in [TEST_EVIDENCE.md](TEST_EVIDENCE.md).
