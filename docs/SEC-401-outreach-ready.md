Subject: RFP: Chain Registry Sepolia testnet security review (4 weeks, Rust + Solidity)

Hello,

We are scheduling a fixed-scope security review of the Chain Registry testnet stack before opening a coordinated public testnet. The engagement targets Sepolia only (no mainnet keys).

Repository: https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-
Scope document: https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/SEC-401-AUDIT-SCOPE.md
Commit / tag for review: `v0.1.1-testnet` (SHA: `203962ff97f2ae103ff44e8bbf9873b6c8e00647`) - pin this ref before audit starts

In scope (priority order):
1. Off-chain package admission and validator pipeline (package_admission, validator_pipeline, publish API)
2. L1 contracts on Sepolia: Staking.sol, Registry.sol, ZKVerifier.sol
3. Operational controls: chain-spec signing, validator set sync, rate limits

Out of scope: Mainnet, cross-chain (cross_chain: false), full ZK soundness proof, governance UI.

Deliverables: Rolling findings (weeks 2-3), final report with severity + PoC (week 4), optional retest window for P0/P1.

Environment we provide: Sepolia RPC access, operator runbook (chain-registry/testnet/OPERATOR.md), optional synced node on public API after HOSTING-301.

Please reply with:
- Earliest start date and team availability
- Fixed-fee or T&M estimate for the scoped 4-week timeline
- Sample smart-contract + systems audit report (redacted)

Thank you,
Samuel Avson

---
Attachments: docs/SEC-401-AUDIT-SCOPE.md (or link above)
After booking: record vendor + start date in docs/NEXT_WORK.md (SEC-401 row)
