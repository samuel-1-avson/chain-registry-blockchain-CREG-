# Client verification

Before installation, the CLI/package-manager integration resolves an immutable
version and verifies:

1. artifact bytes and declared digest;
2. publisher identity and namespace authority;
3. evidence signatures and required profile predicates;
4. transparency-log inclusion and consistency;
5. witnessed checkpoint freshness;
6. policy/profile version and expiry;
7. revocation, dispute, supersession, and deprecation state; and
8. local organization policy.

The client fails closed on stale metadata, missing evidence, failed inclusion
or consistency proofs, digest mismatch, revoked subjects, or ambiguous
`latest` resolution. It never silently falls back to an unqualified artifact.

Install and PATH-shim admission persist a local anti-rollback journal
(`freshness-state.json`, overridable with `CREG_FRESHNESS_PATH`). A previously
observed revocation cannot be replaced by a later verified/unverified/unknown
status, and a qualified record whose timestamp is older than the last accepted
record is rejected as a freeze. `creg cache --clear` wipes both the verdict
cache and this journal. `CREG_FRESHNESS_RESET=true` forgets only the persisted
tree head (emergency recovery after operator-signed head rotation); it does
not forget revocations.

When `CREG_TRANSPARENCY_LOG_PUBKEY` and `CREG_TRANSPARENCY_WITNESS_KEYS` are
configured (or `CREG_REQUIRE_WITNESSED_HEAD=true`), a verified install must
present a witnessed signed tree head, inclusion of the exact subject/digest,
and a consistency proof from the last accepted head. Unsigned
`GET /v1/transparency/.../head` bodies are monitoring data and are not
install-eligible. `--unverified` can skip a missing witnessed head only for
unqualified packages; it cannot override rollback, freeze, or revocation.

TUF-style root, snapshot, timestamp, and targets metadata protects client
updates and rollback/freeze behavior. Cache exceptions require an explicit,
auditable forensic mode. Independent witness operations and a deployed
threshold timestamp role remain operational work; this client gate is the
in-repo enforcement point.
