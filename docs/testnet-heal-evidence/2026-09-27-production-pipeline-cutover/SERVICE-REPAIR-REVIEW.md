# Remaining service and CLI repairs before full release

Reviewed 2026-09-27 against the actual source and live Sepolia inventory;
service/code status refreshed 2026-09-28. These are open requirements, not
completed implementation or simulated evidence.
The faucet funding authorization was executed exactly as approved. No additional
funding, allowance, governance-role or contract mutation is authorized by this review.

## Verified service state

| Item | Actual result |
| --- | --- |
| Faucet | Public healthy service; real 1,000-CREG receipt; database/faucet restart preserves cooldown |
| Validator registration | All four signed-roster EVM addresses return active on fresh Sepolia staking contract |
| Work rewards | Zero packages distributed and zero validator pending balances at the recorded L1 block |
| Reward funding mechanics | `distributeWorkRewards` pulls tokens from treasury; current allowance is max uint256, with sufficient treasury tokens |
| Reward authorization | Only the governance contract or staking contract may call this distribution function |
| Sponsored staking | No public relayer service deployed; durable service acceptance not established |

See `live-service-contracts.json` for exact block, roles, addresses and balances.
An empty rewards-contract balance does not justify another treasury transfer.

## 1. Reward dispatch

Source: `crates/node/src/validator_pipeline.rs` and `contracts/ValidatorRewards.sol`.
The original node hook ran before block finality, supplied reputation 50 for every
validator, and attempted a call from an unauthorized node EOA. The current source
removes that call and records a truthful skipped-emission counter, so premature
L1 sends are disabled. The contract still has no package-level deduplication, and
there is no durable finalized-work outbox, authorized single dispatcher, or receipt
recovery. Rewards therefore remain intentionally unavailable.

Implement and review these steps before activation:

1. Create reward jobs only from an authenticated canonical finalized publication,
   after validating its actual distinct validator signatures and committed committee.
2. Derive contributions, mandatory flags and reputation from the finalized record
   and authenticated authority; reject missing/zero recipient mappings. Preserve
   the actual risk/review verdict instead of replacing it with a fabricated value.
3. Persist the job key (chain identity, finalized block, canonical publication),
   dispatch ownership, approved parameters and transaction before broadcast.
4. Use the existing contract's authorized governance execution path. Prepare the
   exact call and bounded gas proposal for review before using governance keys.
   Do not redeploy a contract or change the signed R7 identity implicitly.
5. Persist transaction nonce/hash/receipt and confirmation state. Recover ambiguous
   broadcasts using the existing transaction and L1 receipt/logs; prevent a second
   dispatcher or retry from creating another payout for the same finalized work.
6. Make metrics distinguish disabled/skipped, pending, submitted, confirmed,
   reverted and failed. Count success only after a successful real receipt.
7. Verify a genuine finalized job and recipient balances, interrupt/restart the
   dispatcher at its persisted boundaries, and prove that the payout is not repeated.

The retired checkpoint bridge remains disabled. A manual reward transaction does
not prove that the durable automatic dispatch pipeline works.

## 2. Sponsored-staking relayer

Source: `crates/relayer/src/main.rs`.
The source now uses a Sled journal with a schema check and one-time legacy nonce
migration. Nonce, daily quotas, and a `submitting` request status are reserved
atomically and flushed before broadcast. Existing corrupt or conflicting state
fails startup rather than resetting to zero. Restart reloads receipt watchers for
submitted/timed-out requests; an interrupted pre-hash submission is marked for
operator reconciliation and its nonce/quota remain reserved. The exact signed
transaction is not persisted, and an ambiguous broadcast does not yet recover
automatically. No persistent production data-dir or real sponsored-stake
receipt/restart acceptance has been verified.

1. Use transactional durable storage for request identity, nonce reservation,
   quotas, exact intent/permit binding, transaction and receipt status.
2. Reserve nonce/quota/request atomically before broadcasting; reject storage
   failure. A missing first-boot store and an unreadable existing store must have
   different handling. Do not reset retained replay state to zero.
3. Reconcile pending/ambiguous transactions on startup and resume receipt checks.
   Preserve status URLs across restart and prevent concurrent duplicate sponsorship.
4. Pin real Sepolia chain/token/staking addresses from the accepted manifest. Use
   a dedicated existing Secret Manager signer after verifying its derived address,
   real gas balance, allowed origins, trusted proxy handling and bounded policy.
5. Deploy privately with persistent storage. Review any necessary exact funding
   proposal, then submit an actual owner-signed publisher permit/intent. Capture
   the receipt and stake increase; restart and prove duplicate rejection and status
   retention. Never substitute a synthetic owner, signature or receipt.
6. Wire the public relayer policy/quote/sponsor/status routes and web/CLI clients
   only after acceptance. Publish an explicit unavailable state while not deployed.

## 3. CLI distribution and proof follow-ups

The corrected scoped-package parser passes a Windows `cargo check` with the Phase C
canonical-transaction feature. The earlier Linux binary predates that fix; a
local Linux release candidate is being rebuilt. No reviewed versioned artifact has
been installed on the real Ubuntu 22.04 host, and no new public CLI release exists.

1. Build and retain the Linux release candidate, then install the reviewed binary
   on the real Ubuntu 22.04 host and verify its version and checksum.
2. Repeat actual installed-binary reads against the candidate. Current revoked
   packages correctly do not produce a passing Merkle proof; use a new genuine
   unrevoked finalized publication for proof/event acceptance before owner revoke.
3. Verify Linux ABI on the existing Ubuntu 22.04 VM. Match archive checksums,
   embedded release versions and signed-spec trust roots for both native platforms.
4. Coordinate final release version/defaults with the existing release history.
   The explicit `0.1.0-testnet-r25` canary must not become an automatic downgrade
   target or cause an updater to select an older incompatible numbered release.
5. Provide usable public platform-specific release metadata/checksums. The current
   updater expects `/v1/releases/cli/latest` or GitHub metadata; the default CDN,
   node record, private GitHub access and each platform's archive must agree.
6. The source installers now stage and validate native binaries/specs before
   replacing an installation. Docker uses a reviewed digest and version, never
   `latest`; macOS ARM must not install an AMD64 native archive. Review remaining
   source-build/config/PATH behavior and advertise only accepted platform paths.

## 4. Recovery and cutover dependencies

The R14 scanner profile is active on all eight real nodes. The fresh R14 package
finalized at h19, its named browser/CLI events were captured, its owner-signed
revoke finalized at h20, and all eight nodes later reported the same h20 tip. The
corrected V4 log audit proves durable local-COMMIT replay before P2P startup and
peer sync. Do not repeat that capture unless a later affected code change makes
the evidence stale.

For active-package proof acceptance, prepare a different genuine canonical npm
package ID that is absent from R7, capture its registry archive and OSV response,
review the exact contents, and include it in a new pinned scanner profile. Roll
the reviewed profile through the four validators and four observers, confirm
shared readiness, then use the corrected CLI to publish, verify finality/proof,
and capture a named event before revocation. This changes only the R7 candidate
chain and is not public cutover evidence by itself.

Full release remains gated by these checks. R27 Explorer is staged on O1 and its
15-route real-data browser capture passed; the public edge still serves the legacy
h0 chain. A frontend proxy/cache repair against that unchanged public chain does
not constitute an R7 chain cutover.
