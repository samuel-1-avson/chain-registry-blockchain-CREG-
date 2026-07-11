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

TUF-style root, snapshot, timestamp, and targets metadata protects client
updates and rollback/freeze behavior. Cache exceptions require an explicit,
auditable forensic mode.
