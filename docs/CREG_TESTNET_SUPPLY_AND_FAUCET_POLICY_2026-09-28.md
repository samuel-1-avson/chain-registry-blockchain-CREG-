# R7 testnet supply and faucet decision

**Decision recorded 2026-09-28:** The target is **42,000,000 total outstanding tCREG on the R7 Sepolia token contract**, with a **separately governed faucet allocation**. The target does not mean placing all 42,000,000 in the public faucet's hot wallet. The faucet allocation amount and replenishment limits remain to be approved.

## Current position

Read-only Sepolia calls at block **11,803,454** (2026-09-28 22:17 UTC) returned:

| Item | Current | Target or rule |
| --- | ---: | --- |
| `CregToken.totalSupply()` | 20,000,000 tCREG | 42,000,000 tCREG total testnet supply |
| `CregToken.MAX_SUPPLY()` | 42,000,000 tCREG | Contract ceiling |
| Capacity available for mint | 22,000,000 tCREG | To be issued through governance if the target is implemented |
| Faucet hot-wallet balance | 8,000 tCREG | Operating balance, not total supply or allocation |
| Existing faucet seed | 10,000 tCREG | Counts toward any eventual faucet program budget |
| Governance contract token balance | 0 tCREG | Candidate custody for the newly issued reserve |

Current token: `0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6`. Token owner/governance: `0x0AB4Db22A9902B8Ef678aBd408889af22277d24E` (2-of-3 signers). Faucet wallet: `0x721C8dBD8208CA9dBdcC12AE9339225458608B81`. The approved seed and first claim are documented in [funding evidence](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/faucet-funding-receipt.json) and [claim evidence](testnet-heal-evidence/2026-09-27-production-pipeline-cutover/faucet-real-drip.json).

## Implementation path

1. **Approve the issuance and custody proposal.** A 2-of-3 Governance proposal can call `CregToken.mint(recipient, amount)`. The recommended recipient for the new reserve is the **Governance contract itself**, so later distributions require governance proposals. At the snapshot, the exact amount to reach the target is **22,000,000 tCREG**. Re-read supply immediately before submission; if supply changes, calculate `42,000,000 - totalSupply` again. The currently deployed token uses `totalSupply + mintAmount <= MAX_SUPPLY`; no token redeployment or CREG-chain reset is needed just to reach this target. [Mint implementation](../chain-registry/contracts/CregToken.sol#L88); [proposal execution](../chain-registry/contracts/Governance.sol#L339).
2. **Approve a faucet allocation separately.** Define `F` as the **total lifetime testnet funding cap for the faucet program**, including the already approved 10,000-token seed. Track tokens actually transferred to users separately. Decide the maximum hot-wallet float, individual claim size, per-address lifetime limit, daily outflow, replenishment tranche, and authorized governance signers. The current 8,000-token wallet balance is not an allocation decision. A governance-controlled reserve can transfer approved tranches to the hot wallet; the public faucet should never custody the entire 42m supply.
3. **Apply technical budget controls before increasing faucet funding.** Reconcile every transfer from ERC-20 logs; persist claim and budget counters; enforce a post-claim balance limit; rate-limit by trusted client identity; alert on low ETH gas and reserve balances. The current service sends 1,000 tCREG per claim, has a 60-second cooldown, and sends zero Sepolia ETH. [Faucet implementation](../chain-registry/crates/faucet/src/main.rs#L83); [audit findings](CREG_TOKENOMICS_DEEP_DIVE_2026-09-28.md#L109).
4. **Update public disclosures after execution.** The whitepaper currently describes 22m as *unminted*. When a governance mint is confirmed, change that to *minted and governance-custodied reserve*, record the proposal ID and transaction hash, and publish the approved faucet budget and current disbursements. Keep the 20m initial allocation and any already staked or claimed tokens in the supply accounting. [Current whitepaper](WHITEPAPER.md#L627).

## Accounting invariant

`totalSupply = sum(all live ERC-20 balances) <= 42,000,000 tCREG`. The faucet wallet balance is one term in that sum, and tokens staked or already claimed remain part of total supply. A faucet wallet holding all 42m would require every other address and contract to hold zero, which is inconsistent with a functioning staking testnet. The token's `burn` function lowers outstanding supply and can reopen mint headroom; a permanent cumulative 42m issuance ceiling would require a separately reviewed contract change. [Token accounting](../chain-registry/contracts/CregToken.sol#L124); [ERC-20 supply and balances](https://ethereum.org/developers/docs/standards/tokens/erc-20/).

### Reviewable issuance call at the snapshot

If supply remains 20m, the proposed Governance `submit` target is token `0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6`, and its call data for `mint(governance, 22_000_000 * 10^18)` is:

```text
0x40c10f190000000000000000000000000ab4db22a9902b8ef678abd408889af22277d24e0000000000000000000000000000000000000000001232ae63c59c6bd6000000
```

This call would mint the remaining 22m into Governance custody after two signer approvals. Its exact amount must be recalculated against the then-current total supply before any proposal is submitted.

**Execution status:** This document records the user's target and a governance implementation path. It does not record an approved faucet allocation amount, a submitted governance proposal, or an executed mint.
