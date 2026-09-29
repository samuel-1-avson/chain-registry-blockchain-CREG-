# R28 testnet faucet working balance and claim policy

**Status (2026-09-29):** The bounded policy is live on the real public faucet as R29, and public token distribution is open. The approved 92,000 tCREG top-up was confirmed, imported at a finalized watermark, and classified in the production ledger. One real 100 tCREG claim passed on a private candidate, survived its restart, and was imported before public promotion. Public health, claim policy, wallet balance, and supply reconciliation all passed after release. No new token was minted.

## Policy staged on the live service

| Control | Staged value | Meaning |
| --- | ---: | --- |
| Standard claim | 100 tCREG | One current minimum validator stake; larger real-test needs need separately reviewed grants |
| Cumulative claim ceiling | 1,000,000 tCREG | Durable confirmed, submitted, and reserved faucet claims count against this ceiling; 2,000 tCREG was already distributed under the previous 1,000-token drip |
| Daily claim ceiling | 50,000 tCREG | At most 500 standard claims in one UTC day |
| Per-address lifetime ceiling | 100 tCREG | One new standard claim per address; prior 1,000-token claim recipients have already exceeded it |
| Faucet working balance limit | 100,000 tCREG | The service rejects claims if its actual wallet balance exceeds this configured float |
| Proposed refill trigger | 25,000 tCREG | Operator prepares an exact top-up back to 100,000, then obtains separate authorization |
| Maximum recipient token balance | 1,000 tCREG | A proposed claim cannot put a wallet above this balance |
| Address / IP cooldown | 86,400 / 300 seconds | Additional abuse controls alongside proof of work and durable lifetime/daily caps |
| Native Sepolia ETH grant | 0 | Faucet wallet ETH pays its own transactions; user gas grants remain disabled |
| Explicit distribution gate | **Closed** | Numeric limits alone cannot open claims; the reconciled ledger and an operator release are also required |

The `FAUCET_TOKEN_ALLOCATION_WEI` setting currently enforces a **cumulative distribution ceiling** on claims. It is not a contract escrow or an automatic 1 million-token mint, and it does not authorize future treasury transfers. Each top-up remains a separate on-chain funding decision. The old 10,000-token seed funded the initial wallet; two historical 1,000-token claims and the new private 100-token acceptance claim count against this staged distribution ceiling, leaving 997,900 tCREG of lifetime claim capacity. After that claim, the live wallet holds 99,900 tCREG, enough for 999 additional standard claims if public distribution opens and other policy limits permit them.

## First working-balance tranche

The [real Sepolia proposal](testnet-heal-evidence/2026-09-29-r28-faucet-top-up-proposal.json) was prepared at block **11,810,086**. It specified a treasury transfer of **92,000 tCREG** to faucet wallet `0x721C8dBD8208CA9dBdcC12AE9339225458608B81`, increasing its balance from 8,000 to 100,000 without changing total token supply. It included exact ERC-20 calldata, source and recipient, code hash, nonce at preparation, and a maximum gas budget of **0.001 Sepolia ETH**. The user approved that exact transfer. The [execution receipt](testnet-heal-evidence/2026-09-29-r28-faucet-top-up-execution.json) records transaction `0x11f797c1d4514973ee88770f359ce63dcafa22c27de8996c909f12d9f713df21` at Sepolia block **11,810,180**. The signed hash was saved before broadcast, the receipt passed two confirmations, and exact-block reads showed **100,000 tCREG** in the faucet, **19,897,590 tCREG** in treasury, and **20,000,000 tCREG** total supply. Actual gas cost was **0.000036824453043451 ETH**, within the approved limit. The [read-only refill status](testnet-heal-evidence/2026-09-29-r28-refill-status.json) was captured before this transfer and is now historical.

The execution script is [execute-approved-r28-faucet-top-up.cjs](../chain-registry/testnet/phase-c/execute-approved-r28-faucet-top-up.cjs). It was run once under exact transaction approval. It checks the expected token, source, recipient, calldata, nonce, balances, gas bound and signer identity; it saved the signed transaction hash **before broadcast**, then read exact-block post-transfer balances. The immutable receipt rules out a second attempt of this tranche.

## Release sequence

1. ~~Review and authorize the exact 92,000-token Sepolia transfer and maximum 0.001 ETH gas.~~ Approved and confirmed; keep `FAUCET_TOKEN_DISTRIBUTION_OPEN=false`.
2. ~~Broadcast once, verify the receipt, import the funding `Transfer` at a finalized watermark, classify it, and reconcile the production ledger.~~ Done. The [classification report](testnet-heal-evidence/2026-09-29-r28-funding-classification.json) records the seed and new tranche separately.
3. Confirm the wallet is at or below its 100,000-token working limit, the 1 million cumulative cap still has room, PoW and trusted proxy IP enforcement remain active, and the production database has no unresolved claims. The private candidate met the wallet and ledger gates before its claim; recheck immediately before public release.
4. A private candidate with `FAUCET_TOKEN_DISTRIBUTION_OPEN=true` sent one [real 100-token claim](testnet-heal-evidence/2026-09-29-r29-real-claim.json), verified its Sepolia receipt and exact balance changes, and recovered the confirmed claim after restart. Its finalized import passed at block **11,810,376**; the same digest-pinned image and policy were promoted publicly as R29, retaining the operator pause route.
5. Read [check-r28-faucet-refill.mjs](../chain-registry/testnet/phase-c/check-r28-faucet-refill.mjs) or equivalent real metrics regularly. At or below 25,000, prepare the exact amount to return the wallet to 100,000. Never automate signing from the public faucet process. Raise the 1 million cumulative limit only after reviewing observed demand and the 42 million supply program.

## Evidence and current limits

The [R28 production rollout report](testnet-heal-evidence/2026-09-29-public-faucet-r28-rollout.json) records the original production PostgreSQL backup, unchanged database container and volume, private restart recovery, and the digest-pinned image. The [history repair report](testnet-heal-evidence/2026-09-29-r28-history-source-repair.json) shows the archive-source repair and the imported 92,000 tCREG funding transfer. The [R29 public release report](testnet-heal-evidence/2026-09-29-r29-public-faucet-release.json) records the finalized claim import, second database backup, preserved production database and volume, enabled bounded policy, and reconciled 20,000,000 tCREG total supply at safe block **11,810,376**. A separate [live verification snapshot](testnet-heal-evidence/2026-09-29-r29-live-verification.json) records public HTTP 200, healthy `/health`, enabled claim policy, **99,900 tCREG** in the faucet, **2,100 tCREG** distributed, and zero unresolved claims. The original R28 browser check predates the real funding and claim.
