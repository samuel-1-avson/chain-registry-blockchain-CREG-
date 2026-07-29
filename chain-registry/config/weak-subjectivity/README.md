# Weak-subjectivity checkpoint release inputs

`canonical-anchor.example.json` is a deliberately non-deployable template.
Its `REPLACE_...` values are rejected by the release gate.

An independent operator prepares three inputs:

1. the exact signed `AuthenticatedValidatorSetSnapshot` returned by the node;
2. the complete validator set active immediately before that transition, in
   the common wire shape `{"validators":[...]}` (without chain-spec metadata);
   and
3. a canonical-anchor observation based on that operator's own chain view.

The anchor's `block_height` must equal `effective_from_height - 1`.
`block_hash` and `validator_set_hash` must match the checkpoint's prior-block
and prior-set commitments. `canonical_head_height` is the independently
observed head used to enforce the explicitly selected maximum checkpoint age.
`source.evidence_sha256` binds the operator's retained raw observation (RPC
response, block export, or equivalent evidence); it is not a placeholder for
that evidence.

Run `node scripts/verify-weak-subjectivity-checkpoint.mjs --help` for the
fail-closed publication and verification commands. The output evidence embeds
the exact compact checkpoint JSON intended for
`CREG_WEAK_SUBJECTIVITY_CHECKPOINT`.

The evidence-signing key is an independent verifier identity. It must not be a
validator, node, chain-spec, or deployment key. Trust in that verifier key is
established out of band; an evidence record's embedded key is never
self-authorizing.
