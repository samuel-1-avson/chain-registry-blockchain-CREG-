# Phase C transaction hashing and queue retirement review

Date: 2026-09-26

## Status (2026-09-26, h6)

All eight actual R7 nodes run R18 and share independently certified h1–h6.
V3's original authorization nonce was already committed; its terminal payload
was atomically retained in dead letter. V4 and earlier full committed-state
restart checks passed. Same-canonical stale PBFT rounds also cleared after R18
validated the real canonical COMMIT certificates.

The stopped-copy review through h5 accounts for nine dead letters: V1=3,
V2=2, V3=2, V4=2, observers=0. Every record reports its publisher nonce already
applied; no terminal record was requeued or deleted. New h6 records await the
next stopped-copy review. Evidence: `dead-letter-review-r18.json` in
`testnet-heal-evidence/2026-09-26-phase-c-events-r16/`.

Two additional live gaps were found. Observers retain package admissions after
publication because their pipeline never performs validator cleanup. R19
compiled certified canonical retirement with durable atomic persistence.
The actual h7 revocation is retained on V1 but has no signed-intent propagation
path. R20 combines R19 with a durable revocation outbox, signature/authority
verification on peers and retry delivery IDs. R20 is compiling, undeployed.
Its acceptance must resolve the exact authorized work identity without operator
queue edits or signing-journal manipulation. Prepared/crash and quorum-loss
recovery gates remain open; public cutover is pending.

## Findings and repairs

| Finding | Source repair |
| --- | --- |
| HashMap serialization made durable work IDs and Merkle leaves change after reload | One feature-selected sorted JSON encoder supplies both identities |
| View-change and reproposal signatures serialized prepared transactions and nested proofs with the same unordered maps | Apply the same encoder to the full signature payload, including nested prepared values |
| The historical spec writer did not escape U+0010 through U+001F correctly and was not a complete RFC 8785 implementation | Use a separate, explicitly named consensus encoding with serde_json scalar/string escaping; preserve historical spec signing bytes |
| Keeping the first nonce variant could retire a valid alternative before checking the retained payload | Validate selected namespace transitions and real package approvals; retain conflicting variants until expiry or canonical nonce application |
| A view or prepared value could arrive between proposer selection and queue cleanup | Recheck captured tip, certified view, live round and durable prepared/COMMIT lock under the node state write lock; repeat before starting a new proposal |
| Queue persistence, retries, terminal retirement and block acknowledgement used asynchronous disk writes | Use synchronous writes; terminal evidence and active-work deletion share an atomic batch, as do block insertion and its acknowledgements |
| Repeated notification or forced delivery could resurrect terminal work or bypass retry deadlines | Notify only newly inserted work, reject ordinary reinsertion of terminal work, and consult stored retry/terminal status during selection and forced delivery |
| A mismatched queue key was silently ignored during hydration | Phase C hydration checks the stored key against the decoded transaction hash and reports an explicit recovery error |
| A proposer or voting peer could accept a publish that later failed the observer's package approval checks | Reuse the observer/sync package-signature verifier before proposal and before a peer starts voting |

The queue never treats equal nonces as proof that two payloads are identical.
Exact transaction duplicates are removed by stable transaction hash. Competing
payloads remain queued while the first valid candidate is proposed. If another
node commits that authorization, the pending variant is retained as terminal
evidence even when its local transaction metadata differs from the committed
record. Retirement is never described as inclusion.

## Encoding contract

The Phase C feature selects `creg-sorted-json-v1`:

- Convert the serializable payload to a serde_json value.
- Recursively sort object keys using Rust string ordering.
- Preserve array order, including transaction and proof order.
- Encode strings and scalar values using the pinned serde_json implementation,
  with compact JSON and no whitespace.
- Hash the resulting UTF-8 bytes with SHA-256, returning lowercase hexadecimal.
- Use this transaction hash as the Merkle leaf and durable work ID. The existing
  internal Merkle rule remains: duplicate an odd final leaf and hash the
  concatenation of the two hexadecimal child strings.

This is a versioned CREG encoding, not a claim of RFC 8785 compliance. The
existing signed-spec encoding remains separate. Ordinary builds retain direct
serde_json transaction and PBFT proof payload encoding. The fresh actual-genesis
marker now names the sorted JSON version, so the new chain has a different PBFT
parent domain from the old candidate. Existing candidate data must not be
reopened as this chain.

## Prepared-value boundary

Queue selection only runs when the captured height/parent has no live proposal,
no prepared value in its certified view, and no durable prepared or COMMIT lock.
The prepared reproposal path uses the original transactions and header. It does
not apply wall-clock expiry cleanup or select a replacement transaction batch.
Any reproposal failure retains the prepared transactions without consuming the
ordinary delivery retry budget or promoting that protected value to dead letter.

The new guard reads the durable lock for the specific next height/parent;
historical completed heights do not block later queue selection. Prepared-state
recovery and certified reproposal still require their separate Step 2 review and
real-host acceptance. This batch does not claim those gates are complete.

## Checks and outstanding evidence

Source review and compilation completed:

- Read hashing, queue hydration, forced delivery, retry, namespace validation,
  package verification, prepared-journal and proposal paths.
- Rust parser/formatter inspection of all changed Rust files succeeded using
  stdout-only output; existing files were not reformatted.
- `git diff --check` succeeded for the reviewed Rust files.

The source-review turn did not run tests; later release builds and real-node
publication, revocation and committed-state recovery checks are recorded
separately. V3's original queue-retirement failure is resolved. In-flight
prepared recovery and review of the additional h5 dead letters remain open.

Still required:

1. Confirm V3's original work ID remains preserved in dead letter and inspect
   the additional h5 dead-letter reasons on stopped database copies.
2. Repeat image/profile replacement recovery at h5 while preserving node
   identities and signing journals; the earlier V4 and fleet committed-state
   restart checks have passed.
3. On the real fleet, demonstrate stable work identity across reload/restart,
   valid selection when conflicting nonce variants exist, retry deadlines,
   preserved terminal evidence and no mutation of a genuinely prepared value.
4. Complete prepared-block, certified view-change and quorum-loss recovery on
   four validators and four observers, with three distinct validator votes.

Selecting namespace transitions validates each accumulated prefix. This favors
safe retention and validation during recovery; batching performance remains to
be measured on real workloads before claiming throughput results. Deferred
payloads require operator visibility and are not silently marked successful.

## Relevant source

- `chain-registry/crates/common/src/consensus_encoding.rs`
- `chain-registry/crates/common/src/block.rs`
- `chain-registry/crates/consensus/src/pbft.rs`
- `chain-registry/crates/node/src/block_producer.rs`
- `chain-registry/crates/node/src/chain_store.rs`
- `chain-registry/crates/node/src/finalized_tx.rs`
- `chain-registry/crates/node/src/pbft_safety.rs`
- `chain-registry/crates/node/src/p2p.rs`
- `chain-registry/crates/node/src/sync.rs`

See [implementation plan](PHASE-C-IMPLEMENTATION-PLAN.md) for the remaining
deployment, workflow and acceptance gates.
