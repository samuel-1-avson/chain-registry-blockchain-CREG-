# Review: requested 20 million tCREG in the testnet faucet

**Status:** Historical, unexecuted full-custody proposal. Its balance table is a snapshot from before the later approved 92,000 tCREG working-balance transfer and private 100-token acceptance claim. The current operating direction is the [bounded R28 faucet policy](CREG-FAUCET-R28-BOUNDED-POLICY.md); none of the 20-million hot-wallet routes below was executed.

## Verified position

At Sepolia block **11,810,014** on 2026-09-29, token `0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6` had exactly **20,000,000 tCREG** outstanding. The [read-only proposal](testnet-heal-evidence/2026-09-29-full-faucet-custody-proposal.json) verifies the treasury, faucet, staking, total and 42 million cap at that block. An earlier [holder audit](testnet-heal-evidence/2026-09-29-current-token-holders.json) at block **11,809,855** identifies the two claim recipients. The treasury, faucet, staking and total figures matched between these snapshots; the recipient detail comes from the earlier audit:

| Custody | tCREG |
| --- | ---: |
| Treasury wallet | 19,989,590 |
| Faucet wallet | 8,000 |
| Staking contract | 410 |
| Two claim recipients | 2,000 |
| **Total outstanding** | **20,000,000** |

The treasury cannot transfer 20 million: it owns **19,989,590**. Transferring that entire balance would raise the faucet to **19,997,590**, leaving **2,410** in other custody. The [proposal JSON](testnet-heal-evidence/2026-09-29-full-faucet-custody-proposal.json) contains the exact ERC-20 `transfer(faucet, 19,989,590 × 10^18)` calldata, token code hash, source and destination, and the block used to calculate it. Its nonce, gas and current balances must be checked again before any execution.

## What exact 20 million faucet custody would take

1. **Mint the missing 19,992,000 directly to the faucet:** this is one Governance proposal and leaves the treasury and existing validator stakes in place. The faucet would hold **20,000,000**, while total outstanding supply would rise to **39,992,000**, leaving **2,008,000** before the 42 million cap. The live Governance contract had a **2-approval threshold** and was not paused at the proposal snapshot. The read-only package contains the exact token `mint` and Governance `submit` calldata; neither was sent. This uses almost all remaining issuance headroom and puts half the planned total supply in the public faucet hot wallet.
2. **Move existing treasury tokens and mint a small remainder:** transfer the treasury's 19,989,590 to the faucet, then use the token owner's governance process to mint **2,410** directly to the faucet. This gives the faucet 20 million but increases total outstanding supply to **20,002,410** and empties the treasury. It needs two separate transactions, including a separately reviewed governance issuance. The read-only package includes both exact calldatas; neither was sent.
3. **Preserve 20 million total outstanding:** transfer the treasury's 19,989,590 to the faucet, then obtain voluntary transfers of the remaining 2,000 from the two claim recipients and withdraw/transfer 410 from Staking. The token has no treasury clawback. The staking balance includes validator stakes; removing them can change the active validator set, and validators have a 14-day unbonding period. This path cannot be treated as an immediate faucet funding operation while preserving the live four-validator network.
4. **Keep governance custody:** define a **20 million faucet allocation** while holding most tokens under governed custody and putting only an approved working float in the faucet wallet. This matches the previously chosen 42 million total testnet supply with a governed faucet allocation. It does not put 20 million into the faucet's hot wallet.

Holding 20 million tCREG in the public faucet hot wallet would concentrate loss and misuse risk. **Recommendation:** keep a governed 20 million allocation if that is the intended budget, set a smaller funded float and claim caps, and retain validator stakes. If the actual requirement is 20 million in the faucet wallet, choose one of the exact-custody paths above and explicitly accept its supply/network effects before any transaction.

## Execution gate

No on-chain action is authorized by this preparation. Before execution, approve the exact path, amount, recipient, signer custody, gas ceiling, treasury and governance impact, daily and per-address claim limits, and the intended total supply after execution. Re-read contract code, owner, total supply, balances and pending nonce on Sepolia at that time. Record transaction hashes and verify receipts and holder balances after every separate action. Do not enable claims solely because the wallet is funded.

The [public faucet](https://faucet.testnet.cregnet.dev/) now displays the real distribution from its reconciled ledger and exact-block Sepolia reads. Its [R27 rollout report](testnet-heal-evidence/2026-09-29-public-faucet-r27-rollout.json) and [browser check](testnet-heal-evidence/2026-09-29-public-faucet-r27-browser.json) confirm the site serves HTTP 200, shows 20 million total and 8,000 in the faucet, and keeps claims paused. This release sent no Sepolia transaction.
