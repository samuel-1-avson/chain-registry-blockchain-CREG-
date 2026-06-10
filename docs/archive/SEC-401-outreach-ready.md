# SEC-401 — Send-ready outreach

**Pin for auditors (GitHub remote):** `v0.1.0-testnet` → `245bf5b59341adff6cbf26769360cf30f112f508`  
**From:** Samuel Avson · samuelavson@gmail.com  
**Scope:** [SEC-401-AUDIT-SCOPE.md](./SEC-401-AUDIT-SCOPE.md) · [live testnet](https://api.testnet.cregnet.dev/v1/health)

---

## Send checklist

- [ ] Email **Trail of Bits** (copy below)
- [ ] Email **OpenZeppelin** (copy below)
- [ ] Attach or link `docs/SEC-401-AUDIT-SCOPE.md`
- [ ] When a vendor replies, update [NEXT_WORK.md](./NEXT_WORK.md) booking table

---

## Trail of Bits

**To:** security@trailofbits.com (confirm on [trailofbits.com/contact](https://www.trailofbits.com/contact))  
**Subject:** `RFP: Chain Registry — Rust validator pipeline + Sepolia contracts (4 weeks)`

Hello Trail of Bits team,

We are seeking a **fixed-scope security review** of **Chain Registry**, a decentralized software supply-chain registry on **Ethereum Sepolia** (testnet only — no mainnet keys). The system combines a **Rust validator node** (package admission, multi-stage validation pipeline, PBFT consensus, libp2p gossip, REST/gRPC APIs) with **Solidity L1 contracts** (staking, registry, ZK verifier).

We are particularly interested in your experience reviewing **systems and smart contracts together** — admission bypass paths, validation pipeline integrity, validator vote aggregation, and consistency between off-chain state and on-chain staking / `eth_getLogs`-driven validator-set sync.

**Repository:** https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-  
**Scope:** https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/SEC-401-AUDIT-SCOPE.md  
**Pin at kickoff:** `v0.1.0-testnet` (SHA `245bf5b59341adff6cbf26769360cf30f112f508`)

**Live Sepolia testnet:**

- API: https://api.testnet.cregnet.dev  
- Explorer: https://explorer.testnet.cregnet.dev  
- Spec: https://spec.testnet.cregnet.dev/chain-spec.sepolia.json  

**Priority in scope:**

1. Rust: `package_admission`, `admission_scan`, `validator_pipeline`, publish API, `validator_set_sync`, chain-spec boot/signing  
2. Solidity (Sepolia): `Staking.sol`, `Registry.sol`, `ZKVerifier.sol`  
3. Cross-layer: stake checks vs L1 state, spec substitution, rate limits, hot-key operational model  

**Out of scope:** Mainnet, cross-chain, full ZK soundness proof, governance UI.

**Timeline:** ~4 weeks — kickoff + threat model (week 1), rolling findings (2–3), final report with PoCs (week 4), optional P0/P1 retest (weeks 5–6).

Please share earliest start date, fixed-fee or T&M estimate, relevant **Rust/systems + Solidity** sample reports, and proposed team composition.

Thank you,  
Samuel Avson  
samuelavson@gmail.com

---

## OpenZeppelin

**To:** audits@openzeppelin.com (confirm on [openzeppelin.com/security-audits](https://www.openzeppelin.com/security-audits))  
**Subject:** `RFP: Chain Registry Sepolia — Staking, Registry, ZKVerifier (+ L1/off-chain integration)`

Hello OpenZeppelin Security team,

We are scheduling a **fixed-scope security audit** of the **Chain Registry** testnet on **Ethereum Sepolia** before a coordinated public testnet launch. **No mainnet keys** are in scope.

Chain Registry anchors package lifecycle and economic stake on L1: publishers stake via **`Staking.sol`**, packages are recorded in **`Registry.sol`**, and validation evidence ties to **`ZKVerifier.sol`**. Off-chain validators (Rust) enforce admission and a multi-stage pipeline before PBFT finalization; **validator membership and stake sync from L1 logs** — we want assurance that contract assumptions match what the node enforces.

**Repository:** https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-  
**Scope:** https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/SEC-401-AUDIT-SCOPE.md  
**Pin at kickoff:** `v0.1.0-testnet` (SHA `245bf5b59341adff6cbf26769360cf30f112f508`)

**Sepolia contract addresses:** `chain-registry/testnet/chain-spec.sepolia.json` (`contracts.*`).

**Live testnet:**

- API: https://api.testnet.cregnet.dev  
- Explorer: https://explorer.testnet.cregnet.dev  
- Spec: https://spec.testnet.cregnet.dev/chain-spec.sepolia.json  

**Priority in scope:**

1. **Smart contracts:** `Staking.sol`, `Registry.sol`, `ZKVerifier.sol`, `CregToken.sol`  
2. **Integration:** `validator_set_sync` and admission stake reads vs contract state and events  
3. **Time-boxed Rust:** publish/admission API and rate limits (scope doc §2.1)  

**Out of scope:** Mainnet, cross-chain (`cross_chain: false`), full ZK soundness proof, governance UI.

**Deliverables:** ~4 weeks — rolling findings, final report with severity + PoC, optional P0/P1 retest.

Please reply with earliest start date, fixed-fee estimate (and options for deeper Rust review), sample audit reports, and engagement lead contact.

Thank you,  
Samuel Avson  
samuelavson@gmail.com
