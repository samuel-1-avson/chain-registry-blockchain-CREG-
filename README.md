## GitHub

**Canonical repository:** [chain-registry-blockchain-CREG-](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-) — protocol, testnet, explorer, hub, and ops in one tree. Waitlist SPA: [Creg-waitlist](https://github.com/samuel-1-avson/Creg-waitlist).

---

# Chain Registry (CREG)

A decentralized package registry that replaces single-authority trust (npm, PyPI, Cargo, and similar) with **independent validator consensus**. Packages are content-addressed, analyzed through a multi-stage security pipeline, and finalized by PBFT quorum. Sepolia hosts staking, registry, and governance contracts. This is a **public alpha**, not mainnet, and not operational Byzantine fault tolerance.

**Network:** `creg-testnet-1` on Sepolia · **Phase:** public alpha · **Live status:** [docs/TESTNET_STATUS.md](docs/TESTNET_STATUS.md)

---

## What problem this solves

Supply-chain attacks exploit one compromised maintainer or stolen API token. Chain Registry treats every publish as a **consensus decision**. At scale, analysis is performed by a small mandatory committee (VRF-seeded, stake-weighted, operator-capped) selected at admission time; any validator may contribute voluntarily. Packages reach **verified** status only after quorum of consensus-grade evidence + PBFT block finality. Consumers install against cryptographic verdicts and on-chain committee records, not publisher reputation alone.

---

## How it works

1. **Publish** — A staked publisher signs a package, pins it to IPFS, and submits to the network.
2. **Validate** — Validators run a three-stage pipeline (static → sandbox → deep scan).
3. **Finalize** — A `⌊2n/3⌋+1` PBFT quorum records the verdict on the chain (RocksDB).
4. **Install** — The `creg` CLI reads **verified** status before delegating to npm/pip/cargo shims.
5. **L1** — Sepolia holds staking, registry, and governance. The legacy Groth16 rollup checkpoint is retired; `zk_validation` is off on the public-alpha spec.

```mermaid
flowchart LR
    Publisher[Publisher] -->|IPFS + signed publish| Node[Validator network]
    Node --> Pipeline[Security pipeline]
    Pipeline --> PBFT[PBFT consensus]
    PBFT --> Chain[(Chain store)]
    Chain --> CLI[creg CLI]
    Chain --> L1[Sepolia staking and registry]
```

| Layer | Technology |
|-------|------------|
| Validator runtime | Rust · libp2p · axum |
| Consensus | PBFT · Ed25519 |
| Content addressing | IPFS |
| Smart contracts | Solidity on Sepolia (staking, registry, governance, ZK verifier) |
| CLI | `creg` — publish, install, stake, audit, multisig |
| ZK | Groth16 circuits (Circom) |

---


## Public surfaces

| Surface | URL |
|---------|-----|
| **Waitlist** | https://waitlist.cregnet.dev |
| **Contribution Arena** | https://contribute.cregnet.dev | Separate educational/simulation product; not the protocol |

**Binaries:** [v0.4.7-testnet](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/v0.4.7-testnet) provides signed `creg` and `creg-node` archives for Windows amd64, Linux amd64, and macOS amd64. The public GCP fleet (`api.testnet.cregnet.dev`) is **paused**.

Chain parameters: [`chain-registry/testnet/chain-spec.sepolia.json`](chain-registry/testnet/chain-spec.sepolia.json).

---

## On GitHub

This repo is organized like other public blockchains — **protocol and consensus first**, not product web apps or hosting runbooks.

| What to explore | Where |
|-----------------|-------|
| **Repository** | [chain-registry-blockchain-CREG-](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-) |
| **Whitepaper** | [docs/WHITEPAPER.md](docs/WHITEPAPER.md) |
| **Public documentation** | [docs/PUBLIC.md](docs/PUBLIC.md) |
| **ZK circuits** | [circuits/](circuits/) |
| **Release binaries** | [Releases](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases) |

The alpha waitlist is live at [waitlist.cregnet.dev](https://waitlist.cregnet.dev). Contribution Arena is a **separate** educational/simulation product and is not part of the protocol workstream.

---

## Who this is for

| Audience | Start here |
|----------|------------|
| **Publishers & developers** | [docs/PUBLIC_TESTNET_QUICKSTART.md](docs/PUBLIC_TESTNET_QUICKSTART.md) |
| **Validators & node operators** | [docs/VALIDATOR_ONBOARDING_CHECKLIST.md](docs/VALIDATOR_ONBOARDING_CHECKLIST.md) |
| **Maintainers (what's next)** | [docs/WHAT_TO_DO_NEXT.md](docs/WHAT_TO_DO_NEXT.md) |
| **Auditors & researchers** | [chain-registry/DEEP_DIVE_ANALYSIS.md](chain-registry/DEEP_DIVE_ANALYSIS.md) |
| **Testnet expectations** | [docs/TESTNET_PHASE_SCOPE.md](docs/TESTNET_PHASE_SCOPE.md) |

---

## Quick start (developers)

```powershell
# Windows
.\chain-registry\scripts\install-creg.ps1 -Version v0.4.7-testnet
```

```bash
# Linux/macOS
./chain-registry/scripts/install-creg.sh --version v0.4.7-testnet

# Or build from source
cd chain-registry && cargo build --release -p cli
```

```bash
# Public API may be unreachable while the GCP fleet is paused.
# Check docs/TESTNET_STATUS.md, or run a local node from chain-registry/testnet/QUICKSTART.md.
export CREG_NODE_URL=https://api.testnet.cregnet.dev
creg doctor
```

---

## Repository layout (blockchain core)

This repository follows the same pattern as other open blockchains: **protocol code and public docs are visible**; web apps, waitlist infrastructure, and cloud deploy runbooks are not promoted from the default README.

| Path | Contents |
|------|----------|
| [`chain-registry/`](chain-registry/) | Rust workspace, contracts, testnet compose, CLI — **[start here](chain-registry/README.md)** |
| [`circuits/`](circuits/) | ZK Groth16 circuits (Circom) |
| [`docs/PUBLIC.md`](docs/PUBLIC.md) | Curated public documentation index |

---

## Documentation

| Document | Purpose |
|----------|---------|
| [docs/TESTNET_SCALE_ARCHITECTURE.md](docs/TESTNET_SCALE_ARCHITECTURE.md) | Future scale-topology design — not a current deployment claim |
| [docs/WHAT_TO_DO_NEXT.md](docs/WHAT_TO_DO_NEXT.md) | **Maintainers** — protocol testing, waitlist, validator onboarding |
| [docs/PUBLIC.md](docs/PUBLIC.md) | Public-facing protocol index |
| [docs/README.md](docs/README.md) | Full maintainer doc index |
| [WHITEPAPER.md](docs/WHITEPAPER.md) | Public-alpha technical whitepaper |
| [DEEP_DIVE_ANALYSIS.md](chain-registry/DEEP_DIVE_ANALYSIS.md) | Technical architecture and issue registry |
| [contracts/README.md](chain-registry/contracts/README.md) | Solidity contract status |
| [circuits/README.md](circuits/README.md) | ZK circuit build and verification |
| [CREG-Contribution-Arena/README.md](CREG-Contribution-Arena/README.md) | Separate educational/simulation product — not protocol |

---

## Status (August 2026)

| Milestone | Status |
|-----------|--------|
| Sepolia contracts | Deployed — addresses in `chain-spec.sepolia.json` |
| Specified validator set | Four foundation observer identities (non-voting); two listed bootnodes; not operational BFT |
| Real sandbox engine | Done |
| Public GCP fleet | **Paused (cost)** — [TESTNET_STATUS.md](docs/TESTNET_STATUS.md) |
| Binary release | [v0.4.7-testnet](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/v0.4.7-testnet) (Windows, Linux, and macOS) |
| Public alpha waitlist | Live — [waitlist.cregnet.dev](https://waitlist.cregnet.dev) |
| Contribution Arena | Separate educational/simulation product — [contribute.cregnet.dev](https://contribute.cregnet.dev) |
| Phase 1 remediation (gossip, batch, relays) | Done — Iteration 6 |
| Fresh genesis ceremony | Candidate recorded 2026-08-01/02 — public activation still gated |
| External security audit | Open — freeze candidate `v0.4.3-testnet` |

This is a **public alpha**, not mainnet. See [TESTNET_PHASE_SCOPE.md](docs/TESTNET_PHASE_SCOPE.md) for participant expectations.

---

## Contributing

Build and test from `chain-registry/`:

```bash
cargo test --workspace
cd contracts && forge test
```

Reference issue IDs from [DEEP_DIVE_ANALYSIS.md](chain-registry/DEEP_DIVE_ANALYSIS.md) in pull requests.

---

## License

MIT — see [LICENSE](LICENSE).
