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
