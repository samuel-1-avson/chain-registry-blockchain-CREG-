# Phase C implementation plan

Date: 2026-09-26

## Current progress (2026-09-26, h6)

All eight real candidates passed guarded R18 replacement/startup. Shared h1–h6
hashes, roots and three distinct COMMIT signatures were independently checked.
V3 terminal retirement, V4 and all eight committed-state restarts passed on the
earlier accepted baseline. Nine terminal dead letters through h5 were reviewed
and preserved. The real h6 publication delivered committed browser and CLI
events and passed the CLI Merkle/quorum proof check.

A certified h6 view change retained the original prepared block hash. An
in-flight crash has not passed: the real h7 revocation remained local to V1,
so its prepared-round probe timed out before any process was interrupted.
Observers also retain stale package admissions. R19 retirement compiled;
R20 combines retirement with durable verified revocation propagation and is
compiling. Both are undeployed at this snapshot.

Next: deploy the reviewed combined repair with each node's data, identity and
signing journal preserved. Re-submit the same real owner intent through one
node. Capture observer-backed browser/CLI revocation events, real quorum-loss
and prepared/COMMIT recovery, then verify all eight finality and clear queues.
Remaining real service writes, funding, durable history and public cutover
follow acceptance. See TESTNET-HEAL-STATUS.md for current evidence and blockers.

The following original plan/state is retained as historical context.

## Objective and original state

Repair the remaining consensus and workflow issues, prove the result on four
real validators and four real observers, then complete the authorized public
chain reset and Phase C cutover.

The previous r1–r4 candidate runs reached four active validators, verified spec
signatures and real peer connectivity. They did not finalize a shared h1.
The namespace pre-activation repair and expiry repair ran on the real fleet.
The latest run exposed unstable transaction hashes and duplicate publisher
nonces. The reviewed r7 node/CLI image compiled successfully; runtime recovery
acceptance remains open. All eight real candidate hosts currently report the
same r7 chain, genesis and h4 hash. A live capture at 2026-09-26 13:32 UTC
confirmed all eight processes, peer paths and scanner profiles are ready. V1
currently runs the r13 node API build; the other seven candidate hosts remain
on r9. The public service still points to the old public chain.

The candidate's publication at h1, empty h2 and h3, and revocation at h4 were
committed on all eight nodes. Their block hashes and three distinct COMMIT
signatures per block were independently checked. The new live capture shows
V3 still has one pending block transaction, one durable-work entry and one
forced-inclusion entry. V1, V2 and V4 retain four dead-letter records in total
(2, 1 and 1); their reasons must be reviewed. Prior committed-state restarts
passed on six hosts, but V3's queue check failed and V4's restart was deferred.
The chain data, failed work and signing journals remain preserved.

Fresh Sepolia contracts, four admitted validators with 100 CREG stake each,
and a publisher with 10 CREG stake are already established. Reuse the existing
eight hosts and fresh contract deployment; verify their live state before
bootstrap. No additional VMs or contract deployment is assumed by this plan.

## Required topology and protocol rules

| Slot | Role | Host / location |
| --- | --- | --- |
| V1 | Validator and real package analysis | This laptop |
| V2 | Validator and real package analysis | `creg-validator-1`, `us-east1-b` |
| V3 | Validator and real package analysis | `creg-validator-3`, `europe-west1-b` |
| V4 | Validator and real package analysis | `creg-validator-4`, `asia-southeast1-b` |
| O1 | Observer and public edge | `creg-testnet-vm`, `us-central1-b` |
| O2 | Observer | `creg-observer-vm`, `us-central1-a` |
| O3 | Observer | `creg-observer-3`, `us-west1-b` |
| O4 | Observer | `creg-observer-4`, `africa-south1-a` |

Each host has its own process, persistent data and P2P identity. Four voting
validator identities require three distinct validator signatures for package
approval, block COMMIT and a certified new view. Observers never count toward
these thresholds. A local timeout cannot authorize replacement leadership.
With only two validators available, new blocks must remain unfinalized.

Acceptance uses actual packages, actual signatures, actual storage and actual
host/process failures. Simulated nodes, mock votes, fabricated certificates
and degraded scanner profiles do not count. These placements provide machine
and regional separation within GCP; they do not establish independent operators
or provider diversity. The laptop supplies the fourth separate validator host.

## Implementation order

| Step | Work | Completion evidence |
| --- | --- | --- |
| 1 | Review and finish transaction identity and queue recovery | Stable hashes and safe queue retirement on real stored records |
| 2 | Finish genesis isolation and client compatibility | Distinct actual genesis, rejected old context, matching node/CLI encoding |
| 3 | Establish real analysis readiness and truthful status reporting | Pinned model/feed evidence, enforced sandbox, accurate workflow states |
| 4 | Build and bootstrap one reviewed fleet artifact | Successful build, pinned digests, identical signed spec on all eight hosts |
| 5 | Prove publication, revocation and PBFT recovery | Shared h1/h2 and real restart/failover evidence |
| 6 | Repair the L1 reward path | Authorized, receipt-confirmed, idempotent real reward settlement |
| 7 | Perform the public reset and cutover | Public API, explorer and clients agree with the accepted fleet |
| 8 | Complete the operational handover | Evidence manifest, recovery runbook and current status documentation |

Review Steps 1–3 together before freezing the release image. Basic h1/h2
acceptance precedes failure drills, so a normal publication has an attributable
first block. The L1 reward path stays disabled until Step 6 passes; its status
must be reported as disabled or skipped during the consensus acceptance run.

### Step 1 — Transaction identity and durable queue recovery

Source review and follow-up changes are recorded in
[transaction/queue review](PHASE-C-TRANSACTION-QUEUE-REVIEW.md). The combined
r7 repair image compiled; real-node acceptance for that batch remains pending.

Primary files:

- `chain-registry/crates/common/src/block.rs`
- `chain-registry/crates/node/src/block_producer.rs`
- `chain-registry/crates/node/src/chain_store.rs`
- `chain-registry/crates/node/src/finalized_tx.rs`
- `chain-registry/crates/consensus/src/forced_inclusion.rs`

Tasks:

- Review the staged sorted-key encoding, including nested maps, arrays,
  strings and numbers. Define its exact wire format and compatibility version.
  Do not claim full RFC 8785 compliance without checking the existing helper.
- Use the same transaction encoding for work IDs, Merkle leaves, queue
  hydration, acknowledgements, forced inclusion and peer verification.
- Confirm a real captured record retains its hash through serialization,
  reload and process restart. Queue hydration must not grow the buffer with
  repeated copies of one durable item.
- Preserve expired, already-applied and duplicate unprepared authorization
  variants before retiring them from proposals. Review the selection rule:
  equal authorization nonces alone do not prove two payloads are identical.
  Preserve conflicting payload evidence and validate the retained candidate.
- Keep dead-letter insertion and active-work removal atomic and synchronously
  persisted. A failed write must leave work recoverable.
- Retire matching forced-inclusion entries without describing them as included.
- Keep quorum-prepared blocks immutable. Review concurrent tip/view changes so
  queue cleanup cannot discard a value needed by an active prepared round.
- Ensure a peer's committed publish also retires this node's pending variant
  of that applied authorization, even if local record metadata differs.

Done when: real record reloads preserve work identity; a real expired or
duplicate unprepared publish is retained with a reason and stops blocking
fresh valid work; prepared-value recovery retains the original block hash.

### Step 2 — Genesis isolation and compatibility

The source review and repairs are recorded in
[genesis/recovery/client review](PHASE-C-GENESIS-RECOVERY-REVIEW.md).
The reviewed r7 node and CLI compiled, and the CLI validated the fresh signed
spec and actual block-zero identity. Real-host acceptance remains required. The r7 follow-up
supersedes the queued r6 snapshot; both source manifests are preserved.

Primary files:

- `chain-registry/crates/common/src/block.rs`
- `chain-registry/crates/common/src/chain_spec.rs`
- `chain-registry/crates/node/src/main.rs`
- `chain-registry/crates/node/src/chain_store.rs`
- `chain-registry/crates/node/src/pbft_safety.rs`
- `chain-registry/crates/node/src/sync.rs`
- `chain-registry/crates/resolver/src/light_client.rs`
- `chain-registry/testnet/phase-c/prepare-candidate-spec.ps1`
- `chain-registry/testnet/phase-c/prepare-node-envs.ps1`

Tasks:

- Review the staged actual-genesis binding. The signed spec must determine the
  real block-zero hash used as the PBFT parent and safety-journal identity.
  Changing a signed chain ID while retaining the old constant block zero is
  insufficient.
- Record the consensus encoding/version in the signed deployment specification
  and make unsupported combinations fail at startup. Confirm node, CLI,
  resolver and observer verification agree on the new encoding.
- Reject existing storage from another genesis before serving requests or
  signing. Preserve r1–r4 data and journals as historical evidence.
- Validate that captured old-chain view-change messages, blocks and publisher
  authorizations cannot authorize work on the fresh chain.
- Review recovery of prepared blocks, local COMMIT locks and certified views
  against finalized history. Every signing path must consult durable locks.
- Review journal retention/pruning: durable block plus certificate must precede
  removal of obsolete records. Reorg or restart must not rewind signing state.
- Check the spec generator's L1/L2 variable separation, signature endpoint,
  live finalized roster, duplicate identities and output overwrite guards.

Done when: all eight hosts derive one identical actual genesis from the new
verified signed spec; another spec derives a different parent; mismatched
storage and old signatures are rejected; the new client verifies new blocks.
Legacy-chain tooling keeps its historical encoding.

### Step 3 — Real analysis readiness and truthful reporting

Primary areas: validator pipeline, ML validator, threat intelligence,
sandbox startup, API/metrics and CLI/explorer status.

Tasks:

- Inventory which enabled scanners actually run on all four validator hosts.
  Record nsjail configuration and enforcement, development bypass disabled,
  scanner versions and profile/evidence digests.
- Confirm the actual agreed scanner profile. The current whitepaper describes
  deterministic rule-based scanning; `DeepScanner` has retired the legacy
  ONNX path and uses YARA-X, OSV and content-hash threat intelligence. Missing
  ONNX weights alone are not a blocker for that profile. If a separate agreed
  profile requires inference, pin real model provenance, hashes and schema
  and prove actual inference; fabricated predictions cannot replace it.
- Establish reproducible threat-intelligence snapshots with source, timestamp
  and digest. Seed data alone does not demonstrate complete feed readiness.
  Keep changing live lookups advisory unless the protocol pins their evidence.
- Make package-quorum approval, queued-for-block, canonical publication and
  revocation distinct observable states. A pipeline VERIFIED log alone does
  not establish a canonical block or finality.
- Represent L1 rewards as skipped, submitted, confirmed or failed. An unset
  bridge key must not increment a successful-settlement counter.
- Retain existing signed feature scope for CR-07 authority activation and ZK.
  Their broader activation gates remain required before enabling those features.

Done when: the enabled consensus-grade profile is backed by real required
artifacts on every validator, missing prerequisites fail closed, and public
status accurately describes the stage reached. Detection coverage limitations
are recorded separately from consensus correctness.

### Step 4 — Build and bootstrap the real fleet

Tasks:

- Complete the current build or repair its actual compiler errors. Incorporate
  any further changes from Steps 1–3 into the final candidate before deployment.
- Build node and CLI with the agreed encoding and pinned spec signer. Preserve
  secure nsjail/runtime components. Archive source revision/diff, build logs,
  node/CLI hashes, OCI index/config digests and transfer checksums.
- Protect artifacts and build caches from concurrent cleanup during deployment.
- Create a new signed spec using one real finalized Sepolia roster snapshot.
  Verify four active, sufficiently staked EVM addresses and matching public keys.
- Parameterize launch scripts for the new chain identity, spec URL, image
  digest and fresh data directory. Preserve existing environment files and
  journals; hardcoded r4 launch paths cannot be reused for the new encoding.
- Install the same pinned artifact on all eight existing hosts. Keep candidate
  ports separate from the old public chain until acceptance passes.
- Check role, signature verification, actual genesis, four-member voting set,
  reachable peer/sync paths, L1 readiness and analysis readiness on every host.

Done when: a host inventory and captured startup/health evidence show all eight
independent processes running the reviewed artifact and identical new genesis,
with four validators and four non-voting observers.

### Step 5 — Real-node acceptance

Run these checks in order and preserve hashes, certificates, timestamps,
validator IDs, relevant logs and restart evidence.

| Check | Required result |
| --- | --- |
| Fresh publication | A new benign canonical package ID, real IPFS bytes and a live signed authorization receive three valid package approvals; h1 commits with at least three distinct authorized validator signatures |
| Agreement | All four validators and all four observers report the same h1 hash and package record |
| Revocation | After shared h1, a real publisher-signed revocation commits as h2; all eight nodes report the same h2 hash and revoked state |
| Durable queue recovery | Repeated reload/restart does not duplicate one real queued record; expired unprepared work is preserved and a fresh valid publish subsequently progresses |
| Proposer outage | Stop the elected proposer on its real host; backups buffer work until three real signed view-change votes authorize replacement leadership; restore the host and verify agreement |
| Prepared-block restart | Observe a real three-signature PREPARE certificate, interrupt the validator process after durable persistence, then restart; the replacement carries the exact original prepared block hash |
| Signing recovery | PREPARE, COMMIT and view locks survive restart; the journal prevents a conflicting signed decision at the same protected height/parent/view |
| Quorum loss | With two of the four validators unavailable, the remaining two cannot finalize a new block or authorize a new leader; restoring a third permits safe progress |
| Observer recovery | Restart an observer and confirm independent block/certificate verification and catch-up to the shared tip |

If the prepared-state window is too short to reach reliably, add an explicitly
guarded pause at the real synchronous prepared-write boundary on the isolated
candidate. It must use a genuine quorum certificate and permit a real process
interruption; it must not fabricate votes, bypass validation or reduce quorum.

Use actual signed requests and real waiting/availability controls to reach an
expiry case. A policy-valid configurable authorization lifetime may be needed
in the CLI to exercise this without an unnecessary hour of waiting.

Done when: every row has attributable evidence, all restored hosts converge,
and each observed round either finalizes or has an explained safe hold that
resolves after quorum/connectivity returns. Stop on a conflicting hash,
unauthorized vote or unexplained stuck round; preserve its journal and repair
forward before repeating acceptance.

### Step 6 — L1 reward settlement

Primary files:

- `chain-registry/crates/node/src/validator_pipeline.rs`
- `chain-registry/contracts/ValidatorRewards.sol`
- Related governance/staking interfaces and deployment configuration

Tasks:

- Implement truthful skipped/submitted/confirmed/failed outcomes first.
- Review the authorized call path. The current contract permits governance or
  staking to call `distributeWorkRewards`; a node bridge wallet is not authorized
  merely because its key exists. Implement the intended governance/staking
  route or a reviewed, narrowly scoped authorization change.
- Replace hardcoded reputation 50 and zero-address fallbacks with authenticated
  validator/reputation data. Reject unresolved recipients or missing evidence.
- Tie settlement to durable canonical finality and a stable work identifier.
  Prevent duplicate rewards across validator attempts, retries and restarts;
  implement receipt/finality tracking and durable retry bookkeeping.
- Confirm the performance feedback path is backed by the actual confirmed
  reward result and cannot diverge consensus selection across validators.
- Recheck deployed roles, signer custody and retired exposed addresses before
  enabling the real transaction path.
- Exercise a real authorized Sepolia settlement and a safe retry/restart.

Done when: a finalized package has a successful finalized L1 receipt and
expected events/accounting, a retry pays once, and skipped or failed attempts
never report confirmed rewards. Keep the bridge key unset until this passes.

### Step 7 — Public reset and cutover

Precondition: Steps 1–6 pass for the functionality being released. A deliberately
disabled L1 path may remain outside a clearly documented consensus-only release,
but the project must then continue to report reward settlement as unfinished.

Tasks:

- Refresh the old-chain public tips, hashes, logs and backups before stopping
  old services. Keep old and candidate evidence manifests attributable.
- Stop the old validator/observer services and switch the public edge to the
  accepted fleet using its accepted data and signing journals. Do not wipe the
  accepted chain for a second reset; another fresh genesis would require its
  own acceptance run.
- Update public spec/signature endpoints, API/RPC routing, explorer, faucet
  contract addresses, CLI downloads/installers and operator configuration.
- Verify externally that chain ID, actual genesis, h1/h2, package state and
  certificate data match the accepted fleet. Confirm obsolete keys/contracts
  do not remain the active public service configuration.
- Preserve signing journals if a rollout fails. Once new-chain decisions are
  signed, repair with those records intact; restoring an earlier journal
  snapshot must never authorize a conflicting decision.

Done when: public endpoints and supported clients verify the accepted chain,
all eight real hosts remain healthy, and the old deployment is retired with
recoverable evidence.

### Step 8 — Operational handover and cost control

Tasks:

- Update the heal-status document, fleet inventory, PBFT safety document,
  whitepaper deployment note and system workflow with the observed result.
  Keep earlier runs labeled as historical and failed where applicable.
- Publish the evidence manifest and a runbook for restart, quorum loss,
  prepared-state recovery, queue dead letters and safe image upgrades.
- Document remaining disabled features and model/detection limitations. Mark
  each pending issue complete only against its specific acceptance evidence.
- Track the four new billable VMs during build pauses. For extended idle
  periods, preserve state and stop unused candidate VMs, then restart the same
  hosts for acceptance. Reserved IPs and disks can continue to incur charges.
- Keep the US$200 `fiffi-500411` budget documented as an alert budget. Account
  separately for the existing GCP project; the budget is not a spending cap.

## Next concrete action

The web/CLI explorer repair is tracked in
[EXPLORER-PIPELINE-REPAIR.md](EXPLORER-PIPELINE-REPAIR.md). The public web
repairs are deployed against the unchanged old chain. The reviewed r11 image
contains node monitoring/listing/search repairs and final console corrections;
its build and real-host deployment must be recorded before marking those API
repairs applied. Recovery, real scanner coverage, rewards and cutover gates
below remain outstanding.

The real scanner bundle and combined profile are pinned and mounted read-only
on all eight running r7 hosts. Runtime readiness and shared publication are
established. Continue in this order:

1. Inspect and repair V3's retained durable work after a real restart. Review
   proposer-only retirement, readiness hydration, startup forced-entry recovery
   and the enqueue-after-commit race against the preserved payload. Backups
   should durably retire permanently ineligible unprepared variants, with
   payload/reason retained and prepared/COMMIT guards enforced. Then rerun the
   failed check and complete V4's restart. Publication is h1; revocation is h4.
2. Capture prepared, COMMIT and view-change recovery after controlled process
   restarts, preserving the existing r7 signing journals and accepted blocks.
3. Review the observed liveness issues: content-based gossip duplicate
   suppression rejects identical PBFT retries, and the local timeout can
   advance its lock immediately after a replacement view becomes certified.
   Confirm observer round status agrees with blocks imported through sync.
4. Complete the remaining reward-path and public cutover gates. LLM advisory
   review was skipped in this run and its readiness is not established.

The old public chain remains running until that evidence passes. No public
reset or cutover has occurred.

## Reference evidence

- [Current status](TESTNET-HEAL-STATUS.md#current-source-repair-and-scanner-state-2026-09-26)
- [r9 real scanner/finality/restart evidence](testnet-heal-evidence/2026-09-26-phase-c-scanner-readiness-r9/README.md)
- [r4 real-node evidence](testnet-heal-evidence/2026-09-26-phase-c-candidate-r4/README.md)
- [PBFT safety requirements](PHASE-C-PBFT-SAFETY.md)
- [Fleet inventory](PHASE-C-FLEET.md)
- [Whitepaper](WHITEPAPER.md)
- [System workflow](CREG_SYSTEM_WORKFLOW.md)
