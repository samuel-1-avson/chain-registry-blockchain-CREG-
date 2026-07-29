# Protocol specification (target v1)

## Package identity

`package_id = (ecosystem, normalized namespace/name, immutable version)`.
`artifact_subject = (algorithm, digest, media_type, size)`. A version may have
multiple subjects, but every decision names the exact subject it evaluates.

## Publisher admission

Production submissions use the versioned dual-domain payload from
`common::publish_signature_message_v2`. EVM and Ed25519 signatures cover the
service domain, chain ID, staking contract, package identity, artifact/source/
manifest digests, publisher keys, ownership claim, nonce, issued-at, and
expiry. Namespaces are controlled by an upstream proof, signed delegation, or
explicit recovery event. Legacy Ed25519-only requests are compatibility-only
and must be disabled with `CREG_REQUIRE_PUBLISHER_DUAL_PROOF=true`.

## Claims and states

Claims are independent and event-derived: `submitted`, `identity_verified`,
`ownership_verified`, `source_bound`, `provenance_verified`, `build_verified`,
`reproducible`, `security_scanned`, `committee_attested`,
`profile_qualified`, `evidence_incomplete`, `expired`, `disputed`,
`rejected`, `revoked`, `malicious`, `superseded`, and `deprecated`.

No claim means “safe,” “malware-free,” or “guaranteed secure.”

## Consensus disposition

Validators sign a named profile and bounded disposition. Individual evidence
digests may differ; all evidence remains retrievable. A policy quorum cannot
turn an unavailable worker into approval. The custom PBFT path is research-only
until independently specified and reviewed.

## Authenticated validator-set transitions

`AuthenticatedValidatorSetSnapshot` version 1 is the authority envelope for a
validator-set change. It contains:

- the first block height governed by the next set;
- the hash of the final canonical block governed by the preceding set;
- the hashes of both the preceding and next validator sets;
- the complete next validator set; and
- a witness of distinct Ed25519 signatures from the preceding set.

The JSON object uses these exact version-1 field names and rejects unknown
members:

```text
{
  "schema_version": 1,
  "effective_from_height": u64,
  "previous_block_hash": lower_hex_sha256,
  "previous_validator_set_hash": lower_hex_sha256,
  "validators": ValidatorSet,
  "validator_set_hash": lower_hex_sha256,
  "transition_signatures": [
    {
      "validator_id": string,
      "validator_pubkey": lower_hex_ed25519_key,
      "signature": lower_hex_ed25519_signature
    }
  ]
}
```

Version 1 permits at most 10,000 next-set members and 10,000 witness entries.
The validator-set hash is SHA-256 over the compact JSON serialization of the
complete validator array after sorting by validator `id` and then lowercase
Ed25519 `pubkey`. The serialized member fields are, in order, `id`, `alias`,
`pubkey`, `eth_address`, `stake`, `reputation`, `status`,
`performance_score`, and `contribution_count`. This byte-level definition is a
versioned compatibility rule; changing the member projection or
canonicalization requires a new schema and migration.

The exact UTF-8 transition signing message is:

```text
creg-validator-set-transition-v1
service:creg-consensus
effective_from_height:<decimal-u64>
previous_block_hash:<lowercase-hex>
previous_validator_set_hash:<lowercase-hex>
validator_set_hash:<lowercase-hex>
```

There is no trailing newline. The set hashes commit to the complete preceding
and next sets; the explicit fields bind height and canonical fork anchor. A
receiver MUST reject unknown fields, an empty or oversized set, duplicate
identities or public keys, non-canonical keys, an incorrect set hash, an
unknown or duplicate signer, a signer key that differs from the preceding-set
entry, any invalid declared signature, or fewer than
`floor(2n/3) + 1` distinct preceding-set signers. A valid quorum does not excuse
an additional invalid witness entry.

For live admission, `effective_from_height` MUST equal the receiver's canonical
tip height plus one and `previous_block_hash` MUST equal that tip's hash.
Persisted transitions MUST continue the locally authenticated set. An exact
logical replay is idempotent; a different transition at the same height is a
conflict.

The logical transition identifier excludes the replaceable signature list.
Different valid quorum subsets for the same signed transition therefore
converge on one identifier. Implementations retain a separate witness digest
for audit, but MUST validate the received witness before accepting the logical
identifier.

The logical identifier is lowercase hex SHA-256 of:

```text
creg-certified-validator-set-transition-id-v1
schema_version:1
<exact transition signing message>
```

The witness audit digest is lowercase hex SHA-256 of the compact JSON snapshot
after sorting witness entries by `(validator_id, validator_pubkey, signature)`.
It is not consensus identity and MUST NOT be substituted for the logical
identifier in committee assignments or checkpoint matching.

## Weak-subjectivity bootstrap and rebuild

`CREG_WEAK_SUBJECTIVITY_CHECKPOINT` contains an operator-provisioned,
independently verified transition certificate. It is a local trust decision,
not a privileged P2P message.

A new or rebuilding node MUST satisfy one of these paths:

1. When pre-checkpoint history is available, validate every transition from the
   immutable deployment bootstrap set and require the exact configured
   checkpoint in that history.
2. When retention has removed the preceding history, install the exact
   configured checkpoint object as the trust root, then validate every
   descendant sequentially against its preceding set.

In both paths, the node MUST reject a missing or conflicting checkpoint,
unsigned authority changes, discontinuous set hashes, invalid descendants, and
locally available block anchors that do not match the canonical chain. A
poisoned descendant invalidates the authenticated read; readers may not ignore
an invalid tail merely because they requested an earlier height.

Checkpoint installation is atomic. It may remove a conflicting local tail only
after every retained descendant has been validated against the configured
checkpoint. A failed validation MUST leave the existing history bytes
unchanged. Reorg recovery removes certificates anchored after the canonical
common ancestor; a transition anchored to the common ancestor and effective at
the following height remains eligible.

The exact configured checkpoint can be accepted on a pruned node even when its
preceding set is unavailable locally. A peer message with the same logical
transition identifier but a different witness is ignored without peer penalty:
the pruned receiver cannot authenticate that alternate witness. The node
persists and propagates its configured checkpoint object, not the unauthenticated
alternate encoding.

## Transition transport

Nodes expose certified history through
`GET /v1/validator/set-transitions` and accept authenticated operator
submissions through `POST /v1/validator/set-transitions`. P2P propagation uses
`creg/v1/validator-set-transitions`.

Receiving a message is not acceptance. Before persistence or propagation, each
peer independently applies request-size and member-count bounds, structural and
signature validation, preceding-set quorum verification, next-height and
canonical-tip checks, checkpoint policy, and history continuity. Payload
invalidity may penalize the sender; a receiver-local I/O failure MUST NOT be
misclassified as malicious peer behavior.

Successful persistence MUST occur before rebroadcast or a success response.
An exact logical replay is a no-op. A conflicting logical transition at an
already authenticated height is a protocol error, not a last-writer-wins
update. A process restart or deterministic state rebuild MUST reproduce the
same authenticated history and active snapshot from the same canonical blocks,
checkpoint, and transition bytes.

## Certified committee assignments

A certified committee assignment binds the logical transition identifier,
snapshot height and set hash, canonical fork anchor, public selection
randomness, selector seed, and ordered members. Receivers load the exact anchor
block, derive the expected randomness from its VRF output (or the specified
anchor-hash fallback), and recompute selection from the authenticated snapshot.
They MUST reject member substitution, order changes, attacker-selected
randomness, certificate substitution, and fork-anchor replay.

`CREG_REQUIRE_CERTIFIED_COMMITTEES=true` is fail-closed: package admission MUST
reject an assignment when checkpoint-descended certified authority is
unavailable. Operators MUST drain or migrate legacy witness-sensitive
assignments before enabling this mode. Network-wide activation requires a
published checkpoint, independent verification, and a multi-operator
partition/reorg/long-range rehearsal; repository tests alone do not establish
those operational facts.

## On-chain validator signature domains

Contract-facing validator authorization MUST use EIP-712 typed data and MUST
not rely on validator count as a membership identity.

The Registry verdict domain is:

```text
EIP712Domain(
  name = "CREG Package Verdict",
  version = "3",
  chainId,
  verifyingContract = Registry
)
```

Its exact struct is:

```text
PackageVerdict(
  bytes32 packageKey,
  bytes32 contentHash,
  bool approved,
  uint256 validatorSetEpoch,
  uint8 quorumPct,
  uint64 issuedAt,
  uint64 expiresAt
)
```

`packageKey` is `keccak256(bytes(canonical))`. The signed epoch and quorum
percentage MUST equal the live Staking/Registry values. The signer MUST remain
active. `expiresAt` MUST be greater than `issuedAt`, the interval MUST be no
longer than 15 minutes, and submission time MUST satisfy
`issuedAt <= now < expiresAt`. The implementation MUST reject malformed and
high-`s` signatures. Changing the decision, package, content, chain,
deployment, membership epoch, quorum policy, or validity interval invalidates
the signature.

Staking validator admission uses the existing domain:

```text
EIP712Domain(
  name = "Chain Registry Validator Admission",
  version = "1",
  chainId,
  verifyingContract = Staking
)
```

with:

```text
ValidatorAdmission(
  address applicant,
  uint256 stake,
  uint256 nonce,
  uint256 ruleSetVersion,
  uint256 validatorSetEpoch
)
```

`validatorSetEpoch` advances once whenever an address enters or leaves the
active set, including a remove/add sequence that restores the same validator
count. A relayer MUST fetch the live epoch used for signing and fail closed if
it cannot do so. The contract recomputes the digest from its current epoch, so
a bundle from a preceding membership is invalid.

These tuple/type changes are migration boundaries. Existing research-alpha
Registry/Staking bytecode and any signer/relayer ABI do not upgrade
automatically. Deployment requires new reviewed bytecode, synchronized ABI and
signer rollout, migration/replay rehearsal, and independent contract review.
