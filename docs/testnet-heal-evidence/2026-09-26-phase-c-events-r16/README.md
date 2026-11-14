# R16 real events and recovery acceptance

## Current continuation: accepted R18, h6

`rollout-r18/fleet-rollout.json` records all eight guarded replacements passing.
`fleet-after-r18.json` captured all eight ready with cleared PBFT/work queues at
h5. `post-h6/blocks/quorum-check.json` independently verifies shared h1–h6 hashes,
Merkle roots and at least three distinct COMMIT signatures on every actual host.
`browser-toidentifier-r18.json` and `cli-toidentifier-r18-watch.txt` contain the
real accepted publish, votes, committed block and verification events; the
browser rendered them without errors. `cli-h6-proof-r18.txt` passed the real
signed-spec publication proof check. All eight use the three-package d09a scanner.

`dead-letter-review-r18.json` accounts for nine consumed-authorizations through
h5, preserved without replay. New h6 records remain to be inspected.
`v1-toidentifier-production-redacted.log` shows the original h6 prepared hash
finalizing after a three-vote certified view change. No in-flight crash occurred
in that natural recovery.

The first actual h7 revocation stayed in V1's durable queue. No signed proof was
sent to the selected proposer. `prepared-crash-h7-r18.json` and
`browser-toidentifier-revoke-r18.json` retain the timeout failures; no validator
was stopped by the prepared monitor. Observers also retain pending admissions
already published. R19 compiled retirement; R20 combines it with a durable
verified revocation outbox and is compiling, not deployed. No manual per-node
POST, fabricated vote or queue/journal modification was used to bypass the gap.

Public cutover remains gated. R17 real service compilation passed, but fresh
faucet tokens/relayer gas, service rollout, durable service recovery and remaining
writes are open. All four additional GCP VMs now run actual candidates and still
accrue charges. The sections below retain earlier states.


The baseline is the real eight-host R7 candidate fleet at accepted h5, separate
from the legacy public chain at h0. No public reset or cutover occurred.

## Accepted before rollout

- R12 image alignment and committed-state restarts passed on all eight nodes.
- V3's original authorization was reviewed as already committed and retired
  with its payload preserved in durable dead letter.
- The real escape-html publication finalized on every host at h5. Independent
  hash/Merkle/three-distinct-COMMIT checks passed through h5.
- `fleet-before-r16.json` confirms all eight scanner-ready and active queues
  clear at the current baseline. Additional dead letters await stopped-copy
  review during the guarded rollout.

## Source and build

The node now emits committed block/package outcomes only after canonical
persistence; REST/gRPC/P2P admission and actual package votes use shared event
paths. Web and CLI consumers unpack the event envelope. CLI names, plural/all
filters, UTF-8 framing and stream timeouts were repaired.

The first build failed after separate Docker cleanup removed its reviewed
builder. `build.log` preserves that failure. `full-source-manifest.json` records
the frozen recovery context without environments, private keys or node data.
`builder-recovery-build.log` and `build-attempt-2.log` show successful node,
inspector and candidate CLI release compilation. `image-build.json` pins the
runtime config/manifest IDs and preserved archive hash. Running temporary
helpers retain the builder and rollback/runtime images during acceptance.

`cli-h5-proof-attempt-2.txt` records successful verification of the real h5
publication against the binary-pinned signer and signed R7 spec. The first
invocation lacked a writable CLI cache location and failed; its diagnostic is
retained. The candidate CLI uses the chain's sorted encoding, independently
checks quorum signatures and requires the authenticated spec.

## Real scanner input

`osv-snapshot-candidate.json` contains successful real OSV queries for is-number,
escape-html and toidentifier. All returned zero advisories. Its epoch is
`osv-2026-09-26-phase-c-r10-event-check`; the combined scanner profile is
`d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4`.
The actual npm toidentifier artifact and upstream metadata are in `next-package/`.
It has not been submitted at this baseline.

## Open acceptance gates

1. Guarded R16/profile replacement and stopped-copy dead-letter review.
2. Real browser/CLI submitted, verified and committed-block event delivery.
3. Shared new publication proof and restart/recovery evidence on all eight nodes.
4. Prepared/COMMIT restart, certified view-change and quorum-loss recovery.
5. Remaining wallet/staking/faucet/relay/reward writes and public cutover.

## Canonical round recovery continuation (R18)

The first R16 rollout stopped before replacement: O1's in-memory round remained
in COMMIT for the exact already committed h5. Read-only fleet captures found the
same state on V4, O3 and O4. Their canonical h5 certificate has three distinct
valid COMMIT signatures; the prior failed h5 proposal had only two PREPARE votes.
No unresolved prepared value was discarded. Signing journals remain preserved.

R18 reconciles matching rounds only after canonical persistence and authenticated
COMMIT certificate verification. Periodic sync also performs reconciliation when
there are no new blocks. Future and conflicting rounds stay retained. Restart
guards verify the actual h5 hash, transaction Merkle root and three Ed25519
signatures against the pinned authenticated R7 spec before allowing a stale
canonical round to restart. Post-replacement readiness requires no unresolved
round. `r18-build-attempt-2.log` confirms release compilation; the first compiler
diagnostic is retained in `r18-build.log`. The reviewed full/delta archives and
image pins are in `r18-image-build.json`. R18 artifacts are being staged on real
hosts; deployment acceptance is pending.

The ordinary readiness capture now includes raw PBFT state and checks unresolved
rounds as well as active queues. Earlier `recovery_clear` captures covered queues
only and must not be interpreted as evidence for an in-flight PBFT restart.

## Additional write-service findings

`write-service-diagnostics.json` records actual HTTP failures: the public faucet
restarts with an empty key and no relay container is running on the explorer
host. Existing fresh faucet/relay secret names were identified without reading
their values. The local Vite faucet route returned SPA HTML and relay targeted
an absent local server; their source proxies and API fallback rules are repaired.
Compose now requires a dedicated faucet key/address and selected token contract
before launch. Service deployment and transaction acceptance remain pending.

All evidence uses actual services and packages. No fixture responses or mock
nodes were used. The four additional GCP VMs run real candidate nodes and
continue accruing charges.
