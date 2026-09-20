# Arena protocol changelog

Monthly operator record of Arena-sourced changes to CREG (protocol, spec, or public docs). This is not a token promise, mainnet attestation, or testnet invite list.

Follow-up labels use the form `chain-registry-blockchain-CREG-#N` or a document revision id. They are not outbound repository links.

The repo copy lives at `docs/archive/ARENA_PROTOCOL_CHANGELOG.md`. The hosted copy lives at `CREG-Contribution-Arena/public/arena-protocol-changelog.md`. Keep them identical.

## 2026-09

External validator onboarding path, dual-key identity registration, and Sepolia Staking Hub multisig governance implemented.

| Milestone | Resolution | Follow-up | Note |
|---|---|---|---|
| External Validator Consensus | Specified on `creg-testnet-1` | `chain-registry-blockchain-CREG-#45` | 4 foundation observer nodes (non-voting); no active validators observed at tip 0 (2026-09-19). |
| Sepolia Staking Hub Governance | Multisig-gated admission implemented | `Staking.sol` at `0x3060...` | 2-of-3 multisig (`0x0fC1...`) designated to approve external validator applicants. |
| Dual-Key Identity Binding | Cryptographic EIP-191 + Ed25519 verification | `crates/node/src/api.rs` | `POST /v1/validators/register` enforces ownership proofs of EVM staker and Ed25519 node key. |
| Zero-Clone Operator Tooling | Global `creg` CLI & `creg-node` daemon | PATH integration | Precompiled binary workflows without repository cloning. |
| Technical Whitepaper v0.4.0 | Updated for Sepolia onboarding | `docs/WHITEPAPER.md` | Documents the onboarding path and the observed testnet status. |

## 2026-08

Initial public alpha release and Arena review.

## How this file is updated

1. Review findings whose resolution is `accepted` or `incorporated` and that have a protocol issue or a docs revision.
2. Add or extend the current month’s section. Do not invent unpublished work.
3. Copy the same markdown to `CREG-Contribution-Arena/public/arena-protocol-changelog.md`.
4. Wave 1 testnet invites stay an operator process: export ready profiles from Arena admin, match waitlist waves offline. Arena Functions must not write the waitlist database.
