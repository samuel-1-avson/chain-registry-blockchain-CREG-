# Remediation implementation status

**As of:** 2026-07-10  
**Release posture:** research alpha only; no mainnet, production-security, absolute-safety, or validity-rollup claim.

## Phase 0 status

| Area | Status | Evidence |
|---|---|---|
| ZK package qualification | Disabled in source/config | Registry entry points revert; CLI no longer creates publisher ZK claims |
| Legacy batch checkpoint | Disabled by default | Bridge requires explicit development flags; Registry entry point reverts |
| gRPC admission | Hardened | Uses the same publisher-authenticated admission policy as REST |
| Publisher proof | Implementing | Versioned dual-domain payload and EVM recovery verifier added; CLI signer wiring and replay persistence remain |
| Namespace ownership | Implementing | Chain and pending namespace owner checks added; upstream/delegation records remain |
| Committee assignment | Implementing | Pipeline consumes admission committee; expansion/proof schema remains |
| Evidence quorum | Implementing | Distinct worker evidence may coexist under one profile/disposition |
| Publisher liability | Implementing | Staking reserves per-release bond; contract tests added |
| Pause recovery | Implementing | Paused unpause proposal and emergency revocation paths added |
| Canonical event history | Design approved | Target is witnessed Merkle history; legacy local chain remains migration input |
| Client freshness | Not started | TUF-style metadata and witnessed inclusion remain Phase 4 |
| Durable orchestration | Not started | Pending pool and finalized transaction queue still need transactional backing |

## Verification commands

| Command | Result | Interpretation |
|---|---|---|
| `npx --yes @mermaid-js/mermaid-cli --version` | Passed (`11.16.0`) | Mermaid source rendered successfully |
| Mermaid render of the audit | Passed; 18 SVGs | Report diagrams are portable image assets |
| `cargo fmt --all -- --check` | Inconclusive; Windows runner stalled with no output | Formatting and Rust compilation remain unverified in this environment |
| `cargo check -p common -p consensus -p node` | Inconclusive; Windows runner stalled with no output; an offline retry reported `Blocking waiting for file lock on build directory` | No claim of compile success |
| `forge test` | Not run; Forge unavailable | Solidity tests remain unverified |

## Required next work

1. Run the pinned Rust, Foundry, web, and infrastructure CI in a clean build environment.
2. Finish EVM publisher-signature CLI generation and persistent nonce tracking.
3. Replace file/memory queues with transactional durable jobs and no-drop tests.
4. Specify and implement the witnessed log/client freshness protocol.
5. Obtain independent contract, sandbox, protocol, and supply-chain reviews.
