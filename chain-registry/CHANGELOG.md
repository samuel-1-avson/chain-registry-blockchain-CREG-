# Changelog

All notable changes to chain-registry are documented here.

## [Unreleased] — MVP

### Architecture

- 6-crate Rust workspace: `common`, `cli`, `resolver`, `validator`, `consensus`, `node`
- 5 Solidity smart contracts: `Registry`, `Staking`, `Reputation`, `VRF`, `Governance`
- Multi-stage Docker build with 3-node + IPFS + Anvil compose cluster
- Full Prometheus + Grafana + Alertmanager observability stack

### CLI (`creg`)

- `creg install` — resolve trust verdict then delegate to real package manager
- `creg publish` — sign and submit a tarball to the pending pool
- `creg status` — query trust verdict for any package (JSON or formatted)
- `creg watch` — stream live registry events via SSE to the terminal
- `creg keygen` — generate Ed25519 keypairs for publishers or validators
- `creg stake` — stake tokens via the Staking contract (publisher or validator)
- `creg setup-shims` — install PATH shims for npm / pip / cargo / gem
- `creg remove-shims` — remove PATH shims
- `creg cache` — inspect or clear the local verdict cache

### PATH Shims

- `npm` shim — intercepts `npm install` / `npm i` / `npm ci` / `npm add`
- `pip` shim — intercepts `pip install` / `pip download`
- `cargo-shim` — intercepts `cargo add`
- `gem` shim — intercepts `gem install`
- Zero workflow change: existing commands work unchanged after `creg setup-shims`

### Trust Resolver

- Cache-first verdict resolution (sled, TTL-based)
- Chain node REST API client with 5-second timeout
- Light-client SPV verification (Merkle inclusion proof + header chain)
- `resolve_verified()` for cryptographically-assured verdicts
- Graceful fallback: node unreachable → Unknown verdict (never hard-crashes)

### Validator (Mechanical Consensus)

- **Stage 1 — Static analysis**: AST patterns, Shannon entropy, obfuscation detection
- **Stage 2 — Sandbox execution**: install-hook behavioral analysis vs declared manifest
- **Stage 3 — Reputation assessment**: publisher history, stake size, account age
- Levenshtein typosquatting detector (30+ popular packages per ecosystem)
- All three stages run concurrently via `tokio::join!`
- `final_decision()` combines all three results with well-defined precedence rules

### Consensus (`consensus` crate)

- Full 3-phase PBFT: PRE-PREPARE → PREPARE → COMMIT
- Quorum: ⌊2n/3⌋+1 — tolerates up to ⌊n/3⌋ Byzantine validators
- Validator set with stake-based slashing and auto-ejection at 3 offences
- VRF-based validator selection (SHA-256 seeded Fisher-Yates shuffle)
- Validator reputation tracking (approvals, rejections, false-positives)

### Node (`creg-node`)

- Axum REST API with 11 endpoints
- Persistent sled-backed chain store (blocks by height + hash, package index)
- In-memory pending pool with ready-for-validation selection
- Finalized-tx `mpsc` channel wiring pipeline → block producer
- Background gossip: vote fan-out + block announcement to all peers
- Linear chain sync: catches up lagging nodes from peers on startup + every 10s
- Publisher index rebuilt from chain replay at startup
- Prometheus metrics at `GET /metrics`
- SSE event stream at `GET /v1/events`
- Light-client proof endpoint at `GET /v1/packages/:canonical/proof`

### Smart Contracts

- `Registry.sol` — core package index; re-verifies ECDSA sigs on-chain before writing
- `Staking.sol` — publisher + validator stake; 7-day unbonding; slash pool distribution
- `Reputation.sol` — per-address approval/rejection scoring 0–100
- `VRF.sol` — on-chain verifiable random validator assignment (Fisher-Yates + block hash)
- `Governance.sol` — M-of-N multisig DAO; auto-executes on threshold; no admin keys
- Foundry deploy script with deployment manifest output
- Full Foundry test suite including fuzz tests for staking invariants

### Observability

- Prometheus scrape config for 3-node cluster + IPFS
- 7 alerting rules (ChainStalled, ChainNodeDown, PendingPoolBacklog, etc.)
- Grafana dashboard: 10 panels (height, packages, pending, nodes, rates, divergence)
- Alertmanager routing: Slack + email, critical vs warning, inhibit rules
- `GET /metrics` Prometheus text-format endpoint on every node

### Infrastructure

- Multi-stage Dockerfile (builder → slim runtime, ~50 MB image)
- `docker-compose.yml` — 3 validator nodes + IPFS + Anvil
- `observability/docker-compose.observability.yml` — Prometheus + Grafana + Alertmanager
- GitHub Actions CI: fmt → clippy → build → test → forge test → Docker build → audit → coverage
- `Makefile` with 15+ dev commands
- `.env.example` with all 25+ environment variables documented
