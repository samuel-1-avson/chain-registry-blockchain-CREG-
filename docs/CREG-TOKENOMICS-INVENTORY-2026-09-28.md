# CREG tokenomics inventory — 2026-09-28

**Type:** Read-only operational snapshot. No signer was used and no state was changed.

**Sepolia snapshot:** block **11,803,622**, timestamp **2026-09-28 22:50:48 UTC**.

**R7 API:** `https://api.testnet.cregnet.dev/v1/public/chain/stats`, queried during the same review.
**RPC:** `https://ethereum-sepolia-rpc.publicnode.com`.

The Sepolia contract reads below were pinned to block 11,803,622. The R7 API and faucet HTTP routes are service observations, not EVM calls, and can advance independently. No faucet claim was submitted as part of this inventory.

## Supply and current balances

| Holder / contract | tCREG balance | Notes |
| --- | ---: | --- |
| Total supply | 20,000,000 | `totalSupply()` |
| Maximum outstanding supply | 42,000,000 | `MAX_SUPPLY()`; 22,000,000 headroom |
| Treasury / deployer EOA `0x3A3206D683441AA91cCA7CD5757B1F0ca463213A` | 19,989,590 | No bytecode at this address |
| Faucet EOA `0x721C8dBD8208CA9dBdcC12AE9339225458608B81` | 8,000 | No bytecode at this address |
| Governance `0x0AB4Db22A9902B8Ef678aBd408889af22277d24E` | 0 | Token owner; reserve not yet minted or transferred here |
| Staking `0x61c00EBe7C46f0bcE63942F7E15Bcc434DFC9B7C` | 410 | Four active validators each hold 100 tCREG in the contract; 10 tCREG remains unattributed by those validator entries |
| Validator 1 EOA `0x91e4dfc18bf015051676f00f23782d0770b95a7a` | 1,000 | Recipient of one confirmed faucet transfer |
| `0x5f424b4ec88073fd461eb194833681a31adfa311` | 1,000 | Recipient of one confirmed faucet transfer; not in the current R7 validator set |
| ValidatorRewards | 0 | No token escrow balance |
| PinningRewards | 0 | No token escrow balance |

These observed balances add to 20,000,000 tCREG. This reconciles the current outstanding supply, but it does not establish legal ownership or vesting for the four named initial allocation buckets. The contract constructor and the fresh deployment script passed the same deployer EOA for each bucket; the live balance shows those funds remain substantially commingled.

## Governance, staking, and incentive state

- The token owner is the Governance contract. Governance is **2-of-3**, active (`systemStatus = 0`), with five proposals recorded at the snapshot. Its public signer list is `0x6323F9780Edf77763CF77401e559711372D76b8C`, `0x5dBD86B868F576e0C5b6dee9f9700A650FCFF639`, and `0x3Febc440Bb85AD51B83576b3Dcc53BD253D75cCc`.
- The Staking contract's owner is the deployer EOA. The four R7 validator-set addresses are each in state `Active` (`2`) with 100 tCREG recorded:

  | Validator | EVM address | Stake |
  | --- | --- | ---: |
  | validator-1 | `0x91e4dfc18bf015051676f00f23782d0770b95a7a` | 100 tCREG |
  | validator-2 | `0x9a33a425953ec36e729e0b7f94b7db0174dbe13b` | 100 tCREG |
  | validator-3 | `0x5470e9700ed487af787af7a2f8c0dc771d5f3fdd` | 100 tCREG |
  | validator-4 | `0xbf10f3553d9ecf43c7bc08a6cff0423a9476892a` | 100 tCREG |

  Together they account for 400 of the contract's 410 tCREG balance. The remaining 10 tCREG needs a holder/liability explanation before any migration.
- ValidatorRewards reports zero packages and zero rewards distributed, and holds zero tCREG. Its treasury is the deployer EOA. The token allowance from that EOA to ValidatorRewards is `2^256 - 1`, so it is an unlimited permission, not an earmarked reward balance.
- PinningRewards reports a zero reward pool and zero distributed rewards, and holds zero tCREG. The deployer EOA has `DEFAULT_ADMIN_ROLE`, `REWARDS_ADMIN`, and `VERIFIER_ROLE`.
- Faucet Sepolia ETH balance is approximately **0.6525145592 ETH**. The current user native-ETH drip is configured as zero.

## Transfer evidence and faucet accounting gap

A read-only token `Transfer` log scan from block 11,780,000 through 11,803,622 returned these three transfers:

| Block | Transfer | Amount | Transaction |
| ---: | --- | ---: | --- |
| 11,795,622 | Treasury EOA → faucet | 10,000 tCREG | [0x39894f356780e065fd9f3d676a458a6d639b8e69a8d7a71b48de860cc530537f](https://sepolia.etherscan.io/tx/0x39894f356780e065fd9f3d676a458a6d639b8e69a8d7a71b48de860cc530537f) |
| 11,795,674 | Faucet → Validator 1 EOA | 1,000 tCREG | [0x02e62d21637b4dbcf4b10522550b09eb367c88447fb6c321ed6ab656f3ca1a0b](https://sepolia.etherscan.io/tx/0x02e62d21637b4dbcf4b10522550b09eb367c88447fb6c321ed6ab656f3ca1a0b) |
| 11,795,961 | Faucet → `0x5f424b4ec88073fd461eb194833681a31adfa311` | 1,000 tCREG | [0xc960b7711f084055f15300be859f2c9275495f8e728883a14cf46e3d544cec91](https://sepolia.etherscan.io/tx/0xc960b7711f084055f15300be859f2c9275495f8e728883a14cf46e3d544cec91) |

At review time the live faucet `/api/stats` returned HTTP 200 and reported 1 drip / 1,000 tCREG distributed, while the token transfer history and current faucet balance show two 1,000-token claims / 2,000 tCREG distributed. The endpoint did not identify its counters as process-lifetime totals. Treat those counters as service telemetry, not a lifetime ledger, until the durable claim ledger is deployed and reconciled.

The faucet `/health` route returned HTTP 200, `healthy`, with token drips available. The funded wallet remains at 8,000 tCREG; that is its present balance, not the approved lifetime faucet allocation.

## R7 public chain observation

The public chain-stats endpoint reported:

- Chain ID `creg-phase-c-testnet-2026-09-26-r7`, genesis hash `0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4`.
- Height 20, 21 blocks including genesis, 8 packages, finalized height 20, and zero finalization lag.
- Four active validators, one publisher, and total R7 stake 400. It reported **two peers**; this endpoint does not prove that four observers are independently running or identify the publisher behind the unexplained 10 tCREG in Staking.
- `l1_block` was 0 while `validator_set_last_finalized_source_block` was 11,803,614. The bridge field said legacy checkpointing is disabled by the Phase 0 safety freeze. Clarify these L1 fields before presenting them as a live Sepolia anchor.

The R7 chain height/stake and the Sepolia EVM block/token balances are different ledgers. Do not display them as one block height or one stake figure.

## Contract code hashes

`keccak256(eth_getCode(address, 11,803,622))`:

| Contract | Address | Runtime bytecode hash |
| --- | --- | --- |
| CregToken | `0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6` | `0xee4ee60af45e022c2673e37e21781b5d88d97164e768ad1d65293122aa1d2b16` |
| Governance | `0x0AB4Db22A9902B8Ef678aBd408889af22277d24E` | `0x0b33803a9ed14a8a70a120957bdf5c97b76335f23c626646f06794dae0c32264` |
| Staking | `0x61c00EBe7C46f0bcE63942F7E15Bcc434DFC9B7C` | `0x593d9af2ca42d8c210b2513408569106a6e13618d00b0fbdea4e4d0adb8062a7` |
| ValidatorRewards | `0xf616699ff68E2a310549fa4280475589B0EB0E49` | `0xc7a08d714c12df96b86bc49213b04f56a480cc68cd25b90fca5cd9cc052740b6` |
| PinningRewards | `0xC35C52315492e5A062f3280829Ba714442216B46` | `0x4cded2f3a43058e09f56c2ccdc20efb3dded51bdce6cb24b69ba9c97485423dc` |

## W0 remaining evidence

- Explain the 10 tCREG in Staking that is not accounted for by the four current validator entries; obtain the publisher identity and current publisher liability data from the authenticated operator view or chain events.
- Reconcile initial constructor issuance and treasury balances against a complete deployment-to-tip event history. The public RPC returned pruned state for historical `eth_getCode` around block 11,780,001, so exact deployment-block lookup requires an archive-capable RPC. Current balance reconciliation is complete at the pinned snapshot.
- Capture the independent eight-host inventory and observer health from the real fleet. Public chain stats only report peer count two.

This is an evidence snapshot, not approval to mint, transfer, or change contracts. Refresh it immediately before any proposal or funding transaction.
