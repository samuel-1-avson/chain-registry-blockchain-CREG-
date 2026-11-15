# Plan: bring validator-1 back (2026-11-15)

**Status:** Option A was carried out on 2026-11-15 and validator-1 runs again on the operator PC. See [the restore record](./testnet-heal-evidence/2026-11-15-v1-restored-on-operator-pc/README.md). The rest of this document is the plan as written before the restore.

**Recommendation:** restore validator-1 on the operator PC with its **original** key, which is still in Google Secret Manager. That needs:
- no new key;
- no Sepolia transaction;
- no Governance vote;
- no change on any other node.

Rotating to a new key or admitting a replacement validator are possible, but both are slower and riskier. Keep them for a real reason: a suspected key leak, or a new independent operator.

## Where things stand

- **validator-1 (v1) has been down since 2026-10-02.**
  - Its container and image are gone from the PC.
  - Its data is intact at `C:\Users\samue\.creg-phase-c\r7\v1\data`: chain store, P2P identity and signing journal. The last write was 2026-10-02 14:57 UTC.
  - Its `node.env` was lost with `C:\Users\samue\CREG-backups`.
- **Only v2 (on the PC), v3 and v4 sign:** 3 of 4, exactly the PBFT quorum. One more outage halts the chain.
- **The original keys are in Secret Manager.** The key manifest (`chain-registry/testnet/archive/fresh-testnet-key-manifest-2026-08-01.json`) records custody in project `gen-lang-client-0022105784`:

  | Secret | Holds | Public value | State on 2026-11-15 |
  |---|---|---|---|
  | `creg-fresh-testnet-validator-1-ed25519` | consensus signing key | `cc4157c4…8ffc`, which is R7 validator-1's registered key | version 1 **enabled** |
  | `creg-fresh-testnet-validator-1-evm` | staking wallet key | `0x91e4dfc18bf015051676f00f23782d0770b95a7a` | version 1 **enabled** |

  Only metadata was checked; no secret value was read. The operator account is project owner.
- **No other node uses validator-1's key.** On 2026-11-15, v2, v3 and v4 had three different keys. The old `genesis2` container on the deleted VM had a key that is not an R7 validator.

## How the code decides who validator-1 is

These findings shape the options:

1. **Membership comes from Sepolia.** The active set is the Staking contract's active wallets, seeded from the signed chain spec. Validator-1's wallet is still active (stake 100).
2. **The consensus key comes from a signed registration.** Each wallet's Ed25519 key is taken from the chain spec, or from the latest registration at `POST /v1/validators/register`. A registration is signed twice: by the wallet (EIP-191 personal sign) and by the Ed25519 key, over this message:

   ```text
   creg-validator-identity-v1
   chain_id:<chain id>
   evm_address:<wallet>
   node_id:<node id>
   ed25519_pubkey:<key>
   nonce:<nonce>
   ```

   A new registration for the same wallet replaces the key, it gossips to the fleet, and each node records the new set from the next height.
3. **Nothing in production creates quorum-certified transitions.** Certified validator-set transitions are verified on receipt, but no production code produces them; only tests do. So a key change goes through the registration path, not a certificate.
4. **The signing journal is bound to one key.** `pbft-safety.rocksdb` stores `{genesis, validator_id, validator_pubkey}` and refuses to open under any other key. With the original key, v1's existing journal opens and keeps protecting against double-signing. A new key would need a fresh journal.
5. **Sepolia-driven changes are stuck.** Validator-set sync reads Staking logs and has been failing on every node ("pruned history unavailable" from the single PublicNode RPC). Any plan that changes the set on Sepolia waits for the RPC fix in `CREG-COMPLETE-DOCUMENTATION.md` §17.2.

## Option A (recommended): restore with the original key

| | |
|---|---|
| On-chain steps | none |
| Signers needed | none |
| Changes to other nodes | none |
| Result | 4 of 4 validators while the PC is on |

Steps:

1. **Confirm the secret is the right key, without showing it.** Read `creg-fresh-testnet-validator-1-ed25519` on the PC, derive its public key, and require `cc4157c4443c9fb011df2b650f497e3eadfc1ddda6dc7f4b6732325383108ffc`. Print only the public key. Stop if it differs.
2. **Confirm nothing else runs that key.** Recheck the key fingerprints of v3 and v4 and of every observer.
3. **Back up v1's data** to a new folder before starting anything, and keep that copy.
4. **Rebuild v1's `node.env`** in a new folder, `C:\Users\samue\.creg-phase-c\r7\v1-private\`, readable only by the operator account and SYSTEM:
   - start from validator-2's file, which uses the same chain, spec URLs, seeds, RPC and scanner profile;
   - set `CREG_NODE_ID=validator-1`;
   - set `CREG_VALIDATOR_KEY` from the secret;
   - take `CREG_OPERATOR_API_KEY` from v2's file or from `creg-fresh-testnet-operator-api-key`.
5. **Start `creg-phase-c-r7-v1`** with the same image and settings as v2 (image `2b472a08…`, privileged sandbox, Docker socket, scanner bundle read-only). Use its own data folder and **different ports**: API `18081`, P2P `9002`.
6. **Check readiness:**
   - the node reports `validator-1` as validator;
   - tip and finalized height 20, hash `730b01c1…`;
   - peers > 0 and the scanner is ready;
   - no "PBFT safety database belongs to a different … identity" error, which proves the journal and key match;
   - 0 ERROR lines;
   - v3 and o4 see its peer ID.
7. **Record it.** Write an evidence folder, update the fleet table in `CREG-COMPLETE-DOCUMENTATION.md` §17, and update the operator memory.

If step 1 or 6 fails, stop v1 (`docker update --restart=no`, then `docker stop`). Nothing else has changed, so nothing else needs rolling back.

**Result and trade-off:**
- With the PC on, all 4 validators sign and the chain survives one cloud outage.
- With the PC off or asleep, 2 of 4 sign and the chain halts. That is the same as today, so this is still a strict improvement.
- Two validators on one machine fail together. Moving one of them to another always-on host later would remove that.

## Option B: rotate validator-1 to a new key

Use this only if the original key may have leaked. For example, if `C:\Users\samue\CREG-backups` was synced or copied somewhere before it disappeared.

| | |
|---|---|
| On-chain steps | none (the registration is off-chain) |
| Signers needed | validator-1's wallet key (`creg-fresh-testnet-validator-1-evm`) and the new Ed25519 key |

Steps:

1. **Generate a new Ed25519 key** on the PC and store it as a new Secret Manager secret before using it.
2. **Sign the registration twice:**
   - the Ed25519 signature with `cargo run -p chain-registry-cli --example sign_validator_identity -- --chain-id creg-phase-c-testnet-2026-09-26-r7 --evm-address 0x91e4dfc18bf015051676f00f23782d0770b95a7a --node-id validator-1 --validator-key-hex <new key> --nonce <unique>`;
   - an EIP-191 signature of the same message by the wallet key.
3. **Submit it** with `POST /v1/validators/register` (operator API key) to one node. It persists the registration and gossips it.
4. **Verify all 8 nodes** list validator-1 with the new public key *before* v1 signs. A node that missed it would reject v1's votes and later fail to verify blocks v1 signed.
5. **Start v1 with a fresh journal.** Move `pbft-safety.rocksdb` aside first, because the old journal is bound to the old key. Then follow Option A steps 4–7 with the new key.

**Risks:**
- This path has not been exercised on R7. Rehearse it on a staging copy first.
- Check that every node also persists registrations it receives by gossip, not only those POSTed to it. Otherwise a restarted node would forget the new key.

## Option C: retire validator-1 and admit a new validator

| | |
|---|---|
| On-chain steps | the new wallet stakes 100 CREG with `applyToBeValidator`; Governance votes 2-of-3 on `approveValidator`; validator-1's wallet calls `initiateUnbonding` |
| Signers needed | the new wallet, two Governance signers (`creg-fresh-testnet-governance-evm`, `creg-phase-c-governance-signer-2-evm`, `creg-phase-c-governance-signer-3-evm`), and validator-1's wallet key |

Why not now:
- It depends on validator-set sync, which is failing (point 5 above).
- During the change, the set can be 5 validators, which need 4 to sign. With validator-1 down, every other validator would have to be up.
- It still needs validator-1's wallet key to unbond, so it is no simpler than A or B.

Use it when an independent operator joins.

## What the operator needs to decide or do

1. **Approve Option A.** Step 1 reads a secret from Secret Manager on this PC; nothing else changes outside the PC.
2. **Say what happened to `C:\Users\samue\CREG-backups`.** If you deleted it on purpose, Option A is clean. If it may have been copied or synced elsewhere, plan Option B afterwards.
3. **Keep the PC on and awake**, and set Docker Desktop to start at sign-in.
4. **Decide about the v2 snapshot.** The manifest lists validator-2's key in Secret Manager as well (`creg-fresh-testnet-validator-2-ed25519`, public key `779c9522…`, the registered one). Once that is confirmed, the GCP snapshot `creg-validator-1-v2-moved-20261115` is a second backup rather than the only one, and can be deleted.
