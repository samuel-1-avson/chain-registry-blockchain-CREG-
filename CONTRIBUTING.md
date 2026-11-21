# Contributing to Chain Registry (CREG)

All contributions — code, docs, tests, reports — are welcome. By contributing
you agree to the [CODE OF CONDUCT](CODE_OF_CONDUCT.md) and the [MIT License](LICENSE).

## Where things live

| Area | Path |
|------|------|
| Rust workspace (node, consensus, validators, CLI) | [chain-registry/](chain-registry/) |
| ZK circuits (Circom / Groth16) | [circuits/](circuits/) |
| Documentation index | [docs/README.md](docs/README.md) |
| Whitepaper | [docs/WHITEPAPER.md](docs/WHITEPAPER.md) |
| Architecture and issue registry | [chain-registry/DEEP_DIVE_ANALYSIS.md](chain-registry/DEEP_DIVE_ANALYSIS.md) |
| Maintainer progress brief | [docs/PROTOCOL_PROGRESS.md](docs/PROTOCOL_PROGRESS.md) |

The Contribution Arena (`CREG-Contribution-Arena/`, `contribute.cregnet.dev`)
is a separate educational product — file those issues against that subtree,
not the protocol.

## Getting set up

```bash
# Build the Rust workspace
cd chain-registry
cargo build --release -p cli

# Run the public testnet API against your local build
export CREG_NODE_URL=https://api.testnet.cregnet.dev
creg doctor
```

Windows helpers:

```powershell
.\chain-registry\scripts\install-creg.ps1 -Version v0.4.8-testnet
```

For validator/operator flows, see
[docs/VALIDATOR_ONBOARDING_CHECKLIST.md](docs/VALIDATOR_ONBOARDING_CHECKLIST.md)
and [chain-registry/testnet/QUICKSTART.md](chain-registry/testnet/QUICKSTART.md).
Note the current network is a public alpha run by the maintainer — running a
validator requires coordination with the maintainer first.

## Workflow

1. **Fork** the repo and create a branch: `git checkout -b <type>/<short-desc>`
   (e.g. `fix/rpc-pruned-history`, `docs/faucet-policy`).
2. **Make the change.** Keep it small and focused; one concern per PR.
3. **Test it** (commands below). Add or update tests for behavior changes.
4. **Open a PR.** Reference the relevant issue ID from
   [chain-registry/DEEP_DIVE_ANALYSIS.md](chain-registry/DEEP_DIVE_ANALYSIS.md)
   when there is one. Describe what changed, why, and how you tested it.
5. **Respond to review.** Address feedback; a maintainer will merge.

Open an issue first for anything larger than a small fix — scope and design
are easier to settle before code exists.

## Build, test, and lint

Run from `chain-registry/` unless noted:

```bash
cargo test --workspace
cd contracts && forge test
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

`cargo deny check` also runs in CI (`deny.toml`). If you add a dependency,
make sure it passes the supply-chain policy in
[chain-registry/config/supply-chain/policy.toml](chain-registry/config/supply-chain/policy.toml).

## Conventions

* **Rust:** `cargo fmt` and Clippy with `-D warnings` must pass. Follow the
  existing crate layout (`crates/<name>/`); don't add a new crate without
  discussion.
* **Solidity:** tests via `forge test`; keep contracts under
  [chain-registry/contracts/](chain-registry/contracts/).
* **Circuits:** see [circuits/README.md](circuits/README.md) for build and
  verification.
* **Docs:** link from an index (e.g. [docs/README.md](docs/README.md)) so new
  pages stay discoverable. One fact, one home — link, don't duplicate.
* **Commits:** short imperative subject, e.g. `docs(testnet): record ...`
  or `fix(consensus): ...`.

## Reporting bugs and security issues

* **Bugs / proposals:** open a GitHub issue with steps to reproduce,
  expected vs actual behavior, and your environment. For network behavior,
  include the chain spec / height observed.
* **Security vulnerabilities:** do **not** open a public issue. Report
  privately through a GitHub private security advisory on this repo so
  disclosures can be coordinated. See
  [docs/SECURITY_OPS_RUNBOOK.md](docs/SECURITY_OPS_RUNBOOK.md).

## Reporting a violation

For Code of Conduct concerns, send a private report via GitHub (DM a
maintainer or use a private advisory). All reports are treated confidentially
and reviewed promptly; see [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
