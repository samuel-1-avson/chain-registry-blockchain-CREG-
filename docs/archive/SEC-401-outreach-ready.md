> **Regenerate before sending.** This file was rewritten 2026-08-22 to drop the retired `v0.1.1-testnet` pin. Confirm the SHA with `.\testnet\prepare-sec-401-outreach.ps1 -Tag v0.4.3-testnet` from `chain-registry/` if a newer tag is cut after VERIFY-401.

Subject: RFP: Chain Registry Sepolia testnet security review (4 weeks, Rust + Solidity)

Hello,

We are scheduling a fixed-scope security review of the Chain Registry testnet stack. The engagement targets Sepolia only (no mainnet keys). This is not a live multi-operator network: the public GCP fleet is paused, and the chain spec lists four coordinated validators with empty bootnodes.

Repository: https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-
Scope document: https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/SEC-401-AUDIT-SCOPE.md
Commit / tag for review: `v0.4.3-testnet` (commit SHA: `3eb3a23d691c5aeafaf866a1d531e518c594fa0e`) - pin this ref before audit starts

In scope (priority order):
1. Off-chain package admission and validator pipeline (package_admission, validator_pipeline, publish API)
2. L1 contracts on Sepolia: Staking.sol, Registry.sol, ZKVerifier.sol
3. Operational controls: chain-spec signing, validator set sync, rate limits

Out of scope: Mainnet, cross-chain (cross_chain: false), full ZK soundness proof, governance UI.

Deliverables: Rolling findings (weeks 2-3), final report with severity + PoC (week 4), optional retest window for P0/P1.

Environment we provide: Sepolia RPC access, operator runbook (chain-registry/testnet/OPERATOR.md). Public API is paused; we will provision a local node or a dedicated operator endpoint.

Please reply with:
- Earliest start date and team availability
- Fixed-fee or T&M estimate for the scoped 4-week timeline
- Sample smart-contract + systems audit report (redacted)

Thank you,
Samuel Avson

---
Attachments: docs/SEC-401-AUDIT-SCOPE.md (or link above)
After booking: record vendor + start date in docs/NEXT_WORK.md (SEC-401 row)
