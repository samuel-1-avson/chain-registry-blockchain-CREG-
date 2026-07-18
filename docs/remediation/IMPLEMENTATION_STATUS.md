# Remediation implementation status

**As of:** 2026-07-18

**Release posture:** research alpha only; no mainnet, production-security, absolute-safety, or validity-rollup claim.

## Phase 0 status

| Area | Status | Evidence |
|---|---|---|
| ZK package qualification | Disabled in source/config | Registry entry points revert; CLI no longer creates publisher ZK claims |
| Legacy batch checkpoint | Disabled by default | Bridge requires explicit development flags; Registry entry point reverts |
| gRPC admission | Hardened | Uses the same publisher-authenticated admission policy as REST |
| Publisher proof | Test incomplete | Versioned v2/v3 dual-domain payload, CLI EVM signing, durable nonce replay protection, and signed gRPC issued-at transport are implemented; upstream authority remains |
| Namespace ownership | Test incomplete | Chain/pending owner checks, signed delegate/transfer lifecycle, durable nonces, restart/rebuild, and fail-closed recovery are implemented; npm/PyPI authority adapters remain |
| Committee assignment | Test incomplete | Pipeline reuses one snapshot-certified assignment bound to fork anchor and anchor-derived randomness; mandatory-mode rollout remains gated on a real checkpoint |
| Historical validator authority | Test incomplete | Quorum-certified transitions, independently validated gossip, peer history retrieval, weak-subjectivity checkpoint ancestry, retention, and reorg truncation are implemented; live operator rehearsal remains |
| Evidence quorum | Implementing | Distinct worker evidence may coexist under one profile/disposition |
| Publisher liability | Implementing | Staking reserves per-release bond; contract tests added |
| Pause recovery | Implementing | Paused unpause proposal and emergency revocation paths added |
| Canonical event history | Design approved | Target is witnessed Merkle history; legacy local chain remains migration input |
| Client freshness | Not started | TUF-style metadata and witnessed inclusion remain Phase 4 |
| Durable orchestration | Test incomplete | Pending work and finalized transactions have durable idempotency, bounded hydration, retry/backoff, DLQ, and canonical acknowledgement; process-kill rehearsal remains |

## Verification commands

| Command | Result | Interpretation |
|---|---|---|
| `npx --yes @mermaid-js/mermaid-cli --version` | Passed (`11.16.0`) | Mermaid source rendered successfully |
| Mermaid render of the audit | Passed; 18 SVGs | Report diagrams are portable image assets |
| `cargo fmt --all -- --check` | Passed on the current working tree | Rust sources satisfy rustfmt |
| `cargo check -j 1 -p chain-registry-node --tests --locked` | Passed | Node library and test targets compile; `sqlx-postgres 0.7.4` emitted an upstream future-incompatibility warning |
| Focused CR-04/HI-11 node tests | Passed | Certified assignment, mandatory mode, receiver adversarial cases, two-swarm propagation, rebuilt-node checkpoint, long-range conflict, retention, and reorg truncation execute successfully |
| Arena `npm test` | Passed (126/126) | Canonical document manifests and production content regressions pass |
| `forge test` | Not run; Forge unavailable | Solidity tests remain unverified |

## Required next work

1. Run the pinned Rust, Foundry, web, and infrastructure CI in a clean build environment.
2. Implement and test cryptographic npm/PyPI authority adapters; retain fail-closed recovery until then.
3. Publish an independently verified validator-set checkpoint and complete the live multi-operator reorg/long-range and recovery rehearsal before mandatory rollout.
4. Specify and implement the witnessed log/client freshness protocol.
5. Complete process-kill/chaos and disaster-recovery exercises for durable work.
6. Obtain independent contract, sandbox, protocol, and supply-chain reviews.
