# Sepolia RPC image and observer canary — resume checkpoint

**Captured:** 2026-10-02 15:53 UTC  
**Purpose:** Resume the live RPC and candidate-image work at the exact current milestone.

## Objective and rollout boundary

Finish the pinned node image build, let the self-hosted Geth node finish Sepolia execution/state sync, verify the same finalized block hash and staking-contract log set through all three configured providers, and only then stage and validate the candidate on **one observer**. Do not update any validator until the observer canary and its acceptance checks pass.

The intended three providers are PublicNode, Tenderly, and the self-hosted Geth endpoint. Their URL values and credentials live in Secret Manager and must not be copied into this file, command output, Git, or chat.

## Current milestone

| Gate | State at checkpoint |
| --- | --- |
| Candidate source | Commit `e8f18d5ad3d745c2047bd2794168a72a7695d945` (`Support offline signed chain spec recovery`) is the source selected for the current build. |
| Candidate image build | Cloud Build `f50a2977-aa8b-4d6f-8ae7-5328e9df343c`, project `gen-lang-client-0022105784`, region `us-central1`, is still `WORKING`. Intended tag: `us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node:phase-c-rpc-r32-rust193-e8f18d5ad3d7`. It has not been verified or deployed. |
| Older image build | Build `a77052cf-72a8-4233-bad9-74722feb736d` completed for commit `f19b2e621a92339540ba87a9b60962afcc0aa40c`; this is the older r31 image and is **not** the candidate to stage. |
| Geth recovery | State-only recovery completed after a ready disk snapshot. The Sepolia ancient chain data was retained, and Geth successfully initialized from the freezer. Geth and Lighthouse are running with restart policy `unless-stopped`. |
| Geth sync | Not at head. Latest container log at 15:46:30 UTC reported chain download 54.17% and state download 2.96%, with estimates near 5h and 7h. A subsequent public RPC probe reported Sepolia chain ID `0xaa36a7`, `eth_blockNumber=0x0`, and `eth_syncing=true`. Do not reset or restart it as a shortcut. |
| Public providers | PublicNode and Tenderly both reported Sepolia head `0xb482e7` (11,829,991) and matching hash at the same height checked in the latest probe (`0xb48078`, 11,829,368). That probe was read-only. |
| Three-provider acceptance | **Not passed.** Geth has no usable execution head yet. PublicNode historical staking-log completeness also remains unresolved; see below. |
| Observer canary | Not staged. Intended canary is `creg-observer-3` in `fiffi-500411`, zone `us-west1-b`. |
| Validator rollout / chain reset / public cutover | None performed as part of this milestone. Keep them that way. |

## Geth recovery record

- VM: `creg-sepolia-rpc`, project `fiffi-500411`, zone `us-west1-b`; machine type `n2-standard-4`.
- Data disk: `creg-sepolia-rpc-data`, mounted at `/mnt/disks/sepolia-rpc`.
- Rollback snapshot: `creg-sepolia-rpc-data-pre-state-recovery-20261002`, status `READY`, created before the repair. Do not delete it as part of this work.
- Pinned Geth container image digest: `sha256:6606aa1c7b6354d57581952604e7ab5a49689aeee9aebf668c81b6d3225395f1`.
- Recovery used Geth `removedb --remove.state --remove.chain=false` with the explicit Sepolia data directory. Geth confirmed state removal and skipped ancient-chain removal; the freezer remained present. The database then initialized from freezer data.
- Lighthouse also restarted and connected to Geth. Its beacon sync distance reached zero, but it reported optimistic execution while Geth was still syncing.
- A later SSH-over-IAP attempt timed out, but the public Geth JSON-RPC endpoint remained reachable. Do not infer Geth is stopped from that SSH timeout.

## Provider observations and exact block anchors

The Secret Manager profile is `projects/fiffi-500411/secrets/creg-sepolia-rpc-profile/versions/1`. Its provider roles are PublicNode (`primary`), Tenderly (`fallbacks[0]`), and Geth (`geth_url`, also represented in the fallback list). This profile is staged only; it has not been applied to CREG nodes.

Observed in the latest read-only probe:

- PublicNode and Tenderly both returned chain ID `0xaa36a7`, head `0xb482e7`, and the same hash `0xa46a299a6f3d0249adf366a6047b5d505b8db5be1c02026143b178ab3318213c` at block `0xb48078` (decimal 11,829,368).
- Geth returned chain ID `0xaa36a7`; its head was `0x0`, syncing was true, and it had no block at that anchor.
- A no-key OnFinality endpoint returned the Sepolia chain ID once, then rejected the historical comparison with rate-limit error `-32029`. It is not part of the configured three-provider profile and has not passed provider acceptance.

**Anchor correction for the next run:** use decimal block 11,829,000 = `0xb47f08`, not `0xb48078`. A previous interactive probe accidentally checked 11,829,368 while intending the 11,829,000 checkpoint. That probe is valid only as a PublicNode/Tenderly same-height hash check; it does not establish the intended checkpoint result. Likewise use staking deployment block 11,781,426 = `0xb3c532` and first observed event block 11,781,480 = `0xb3c568`. One attempted OnFinality request used incorrectly transcribed range bounds and was rate-limited before returning data; it caused no state changes and must not be treated as evidence.

Earlier live inspection found Tenderly returned 13 staking-contract logs in 9 event blocks from deployment through the intended 11,829,000 checkpoint, while PublicNode returned zero logs and incomplete receipt data for known transactions. The reported event-set digest was `667595b0c490f92613b371d412ccd0ad8d9834115029240fe50c19c07a1d0fe1`. Because the latest anchor transcription needed correction, re-run both providers with the exact decimal-to-hex bounds above and compare normalized full log records before making a final provider decision. Do not accept a count-only comparison.

Contract under review: Staking at `0x61c00EBe7C46f0bcE63942F7E15Bcc434DFC9B7C`; signed R7 chain spec says Sepolia chain ID `11155111` and staking deployment block `11781426`.

## Candidate image and observer details

- Candidate source commit: `e8f18d5ad3d745c2047bd2794168a72a7695d945`.
- Candidate build tag: `phase-c-rpc-r32-rust193-e8f18d5ad3d7` as recorded above. Poll Cloud Build and capture the successful image digest and version/binary-hash verification before using it.
- Observer candidate: `creg-observer-3` (`fiffi-500411`, `us-west1-b`). It is currently on its older R7 image, has persistent node data and the scanner mount, and has not been changed for this candidate.
- Existing cached chain-spec hash on observer-3: `331897f5a3f89fe902e4a7d814a9245ef50b384e22cc3d8dc946b4b45ed39e9f`. Its detached signature was missing from the old cache. The candidate adds offline signed-spec recovery; before staging, verify the checked-in R7 spec and detached signature pair, copy them to the intended data paths, and configure offline verification explicitly (`CREG_CHAIN_SPEC_OFFLINE=true`, `CREG_CHAIN_SPEC_SIG_PATH=/data/chain-spec.cached.json.sig`). Keep the old node data and scanner mount intact and record rollback image/container details.
- Observer-3's old container was `creg-phase-c-r7-o3` with image ID `sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9`. Recheck this live before any change.
- The prior rollout helper is stale and unsafe; do not use it without review. Observer-3 lacks Artifact Registry read access, so a controlled image transfer over IAP may be needed. Never copy credentials into shell history or evidence.

## Exact continuation sequence

1. Poll build `f50a2977-aa8b-4d6f-8ae7-5328e9df343c` until terminal status. If successful, record its immutable image digest, source commit, build status, `--version`, and runtime binary hash. Do not use r31.
2. Leave Geth and Lighthouse running. Monitor `eth_syncing`, `eth_blockNumber`, peers, and recent logs. Wait until `eth_syncing=false` and the execution head tracks the current Sepolia head. Investigate progress/errors without deleting data or repeating state recovery.
3. Choose a common finalized checkpoint below the three live heads, then query chain ID and block hash at that exact height from PublicNode, Tenderly, and Geth. Require all three chain IDs to equal 11155111 and all three block hashes to match.
4. Query the Staking contract logs from deployment block `0xb3c532` through the same checkpoint on all three providers. Compare normalized complete records (block hash/number, transaction hash/index, log index, address, topics, and data) and verify relevant transaction receipts. If any endpoint is incomplete, throttled, or inconsistent, stop this gate and resolve the provider choice; do not paper over missing logs with a second shared public endpoint.
5. Only after gates 1–4 pass, stage the verified image on observer-3. Preserve its current data and rollback image. Validate the service health, candidate version/hash, signed chain-spec verification, synced chain identity/tip, peer/API readiness, and explorer/metrics paths. Save evidence and compare it with the pre-stage capture.
6. Stop and report for review after the observer acceptance. No validator rollout is included in this checkpoint's authorized scope.

## Repository state and references

The working tree already contains unrelated modifications and untracked evidence, including edits in `chain-registry/crates/node/src/validator_set_history.rs` and `docs/testnet-heal-evidence/2026-10-02-blocker-repair/BLOCKER-REPAIR-STATUS.md`. Preserve them. Do not run `git clean`, reset, or broad staging. This checkpoint is an additional file only; it has not been committed or pushed.

Relevant records:

- [Blocker repair status](BLOCKER-REPAIR-STATUS.md)
- [RPC resilience progress at 14:07 UTC](rpc-resilience-progress-20261002T140747Z.json)
- [Initial Geth startup record](gcp-sepolia-rpc-startup-20261002.json)
- [Current acceptance gates](../../../testing/ACCEPTANCE_GATES.md)
- [Provider review](../../2026-10-01-multivalidator-acceptance/RPC-PROVIDER-REVIEW.md)
