# Phase C genesis, recovery and client review

Date: 2026-09-26. This records source repairs, not fleet acceptance.

## Scope and outcome

Reviewed the signed network identity, persistent vote locks, exact prepared
block recovery, observer certificates and resolver/CLI proof paths. The
existing whitepaper and system workflow require four real voting validators,
three distinct approvals for finality, and four non-voting observers. Those
requirements remain the deployment gate. No node was simulated or deployed
for this review, and the old public chain was not reset.

## Repairs written

| Finding | Repair |
| --- | --- |
| Changing a signed chain ID previously left actual block zero unchanged | Phase C block zero binds the computed signed genesis and the new encoding domain. Signed genesis also commits the complete bootstrap authority, including status and reputation. |
| A build could interpret a spec with the wrong transaction codec | The signed `consensus_encoding` must equal `creg-sorted-json-v1` for the Phase C node and CLI. Historical builds require the field to be absent. The declared genesis must equal its computed payload. Node startup also rejects a common codec/node profile mismatch. |
| Bootstrap identities and local signing key could disagree | Validate unique IDs, Ed25519 keys and distinct nonzero EVM addresses; require at least four active staked members and match the local key to the signed active roster. |
| Missing signing storage could be recreated with no historical vote locks | Persist a synchronous journal identity binding in chain storage. Bound or existing journals open without creation. A nonempty chain cannot bootstrap a missing journal. Missing or empty bound storage stops signing. |
| A crash between the view-change decision and payload writes could lose the original vote | Persist the local signing decision, view floor and complete signed vote in one synchronous batch under the node state write guard. Recovery rejects decisions missing their exact payload. |
| New stale-view COMMITs remained possible after voting to leave that view | Reject new COMMITs below the durable local view floor; the identical previously recorded COMMIT can still be recovered or rebroadcast. |
| A fork parent could bypass a parent-scoped COMMIT lock | Enforce the durable COMMIT across the entire height before PREPARE, COMMIT, prepared persistence, canonical insertion and reorg preflight. Startup checks the canonical parent as well as block hash. |
| Prepared recovery replayed signatures in the original header view | Restore the certified prepared view and reproposal proof before ingesting recovered PREPARE signatures. |
| The producer guard could block authorized same-value reproposal | Permit a verified higher certified view to carry the same protected block hash. A round already operating in that view stays guarded. |
| Reorg checkpoints reopened with historical block zero | Reopen checkpoints with the actual canonical genesis of the source store. Preserve all signing journals; checkpoints do not copy or recreate signing authority. |
| Observer verification rejected signed `active` members and counted inactive members in thresholds | Use the shared active-status rule and count active authority consistently. Phase C observers reject a reduced quorum or an authority with fewer than four active members. |
| Equivalent view-change witnesses caused an idempotent insertion conflict | Compare the hash-bound header and transaction body, excluding replaceable view-change proof material; preserve the first accepted witness. |
| Resolver Merkle leaves, header hashes and validator-set hashes used historical rules | Use common transaction hashes and block-header identity. Phase C validator-set hashes commit the complete authenticated authority. |
| A proof server supplied its own trusted validator list | Phase C clients authenticate the binary-pinned spec signer and bootstrap, then verify ordered preceding-set quorum transitions and their canonical fork anchors. Server-supplied members do not establish trust. |
| A Merkle leaf was not bound to the requested package or download fields | Return the exact transaction and bind its hash, canonical package ID and publication status. Populate positive verdict content hash, CID, publisher, findings and risk summary from that proven transaction. |
| A proof error returned a positive unverified verdict | Resolver verification fails closed. Phase C install lookup obtains a live publication proof before caching positive results. Cached revocation can still block immediately. |

## Artifact boundary

The active r5 build uses an earlier source snapshot. The queued r6 transaction
review was superseded by this recovery/client batch; its original manifest is
retained and must reject changed source. The reviewed follow-up is recorded
separately as `creg-node:phase-c-secure-r7-review` under
`testnet-heal-evidence/2026-09-26-phase-c-genesis-recovery-review`.

The spec generator writes the signed encoding field and incorporates the new
authority commitment. Environment generation rejects old specs. A fresh signed
spec was prepared locally for `creg-phase-c-testnet-2026-09-26-r7`, after
checking all four real Sepolia staking entries at block 11,784,358. Its signed
genesis is `0x1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234`.
The common crate compiled on the host during generation and verified the
signature against the pinned signer. The reviewed Linux CLI then validated
the full spec, signer and encoding, deriving actual block zero
`0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4`.
The spec is unpublished; its signature
endpoint and isolated data directories are still required before starting a
candidate. The r4 spec and journals remain historical
evidence and must not be reused as the new signing domain.

## Checks and remaining gates

Changed Rust sources passed parser-only checks. Local `cargo check --locked`
passed for the resolver and consensus crates with the Phase C common feature;
this also checks the common codec/spec types. No automated tests were added
or run. Node/CLI release-image compilation status is recorded separately by
the build helper: both compiled successfully as
`creg-node:phase-c-secure-r7-review`. The built CLI accepted the authentic new
spec with signature verification required. These checks do not establish
runtime recovery, shared finality or complete scanner readiness.

Still required on the actual eight-host fleet:

1. All hosts derive the identical actual block-zero hash from the authenticated
   new spec; the previous spec/storage and signatures are rejected.
2. Finalize shared publication h1 and revocation h2 with real package analysis
   and three of four validator signatures; all observers sync the same hashes.
3. A real restart retains the exact prepared block, COMMIT and view-change
   payload. Stop a real proposer and prove certified failover without changing
   a carried prepared block hash. Two available validators must not finalize.
4. Exercise loss/corruption of copied signing storage only on isolated preserved
   copies, and verify startup refusal. Never delete a live validator's journal.
5. Verify the CLI against the authenticated candidate spec and real block;
   failures or mismatched download metadata must not yield verified admission.

Publication inclusion is not a proof of the latest chain tip or the absence
of a later revocation. Existing client freshness and witness checks remain
required. Journal records are retained without pruning. Recovery intentionally
stops on a prepared/COMMIT conflict with canonical history; a conflicting
journal is not repaired by wiping locks. Complete loss of both the chain binding
and journal cannot be detected from an empty data directory alone, so preserved
signing storage and an intentional new signed genesis remain operational
requirements. Real scanner readiness and the L1 reward path remain separate
unfinished steps. Public reset and cutover remain pending.

The scanner review clarified an earlier assumed blocker: the current whitepaper
describes deterministic rule-based scanning, and `DeepScanner` has retired
the ONNX path. An empty `models/` directory alone does not establish that the
agreed profile is incomplete. The actual gate is reproducible YARA-X rules,
the agreed pinned OSV/threat-intelligence sources and sandbox evidence on all
four real validators. No model or scanner output was fabricated for this work.
