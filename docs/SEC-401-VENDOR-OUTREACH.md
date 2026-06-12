# SEC-401 — Vendor outreach template

**Scope document:** [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md)  
**When booked:** Record vendor name and start date in [NEXT_WORK.md](./NEXT_WORK.md) (SEC-401 row).

**Send-ready email:** From `chain-registry/`, run `.\testnet\prepare-sec-401-outreach.ps1 -Tag v0.1.1-testnet -ContactName "Your Name"` to generate [SEC-401-outreach-ready.md](./SEC-401-outreach-ready.md). Send to vendors below; record booking in [NEXT_WORK.md](./NEXT_WORK.md).

---

## Email subject

`RFP: Chain Registry Sepolia testnet security review (4 weeks, Rust + Solidity)`

---

## Body (copy and customize)

Hello,

We are scheduling a **fixed-scope security review** of the Chain Registry testnet stack before opening a coordinated public testnet. The engagement targets **Sepolia only** (no mainnet keys).

**Repository:** https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-  
**Scope document:** attached `SEC-401-AUDIT-SCOPE.md` (or link to repo `docs/SEC-401-AUDIT-SCOPE.md`)  
**Commit / tag for review:** `v0.1.1-testnet` (SHA from `prepare-sec-401-outreach.ps1` output — pin before audit starts)

**In scope (priority order):**

1. Off-chain package admission and validator pipeline (`package_admission`, `validator_pipeline`, publish API)
2. L1 contracts on Sepolia: `Staking.sol`, `Registry.sol`, `ZKVerifier.sol`
3. Operational controls: chain-spec signing, validator set sync, rate limits

**Out of scope:** Mainnet, cross-chain (`cross_chain: false`), full ZK soundness proof, governance UI.

**Deliverables:** Rolling findings (weeks 2–3), final report with severity + PoC (week 4), optional retest window for P0/P1.

**Environment we provide:** Sepolia RPC access, operator runbook (`chain-registry/testnet/OPERATOR.md`), optional synced node on public API after HOSTING-301.

Please reply with:

- Earliest **start date** and team availability
- Fixed-fee or T&M estimate for the scoped 4-week timeline
- Sample smart-contract + systems audit report (redacted)

Thank you,  
`_______________`

---

## Suggested vendors (examples — not endorsements)

| Firm | Notes |
|------|--------|
| Trail of Bits | Rust + Solidity, strong systems background |
| OpenZeppelin | EVM contract focus |
| Consensys Diligence | Full-stack Web3 |
| Cyfrin | Solidity education + audits |
| Internal red team | Use same scope doc; record as "internal" in NEXT_WORK |

---

## Checklist before sending

- [x] Engineering lead reviewed [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md)
- [x] Auditor baseline tag proposed: `v0.1.0-testnet` (DIST-301)
- [ ] No production/mainnet secrets in repo or attachments
- [ ] Sepolia RPC endpoint provisioned (read-only or dedicated key)
- [ ] Start date recorded in NEXT_WORK when vendor confirms
