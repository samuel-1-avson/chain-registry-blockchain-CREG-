# Architecture migration plan

New security-significant writes MUST use the versioned submission/evidence/
policy/log schemas. Legacy `Verified` records are historical events labelled
with their original assumptions and MUST NOT become `profile_qualified` by
conversion alone.

1. Freeze ZK qualification and legacy batch settlement; publish a signed
   research-alpha capability notice for every running environment.
2. Introduce a canonical event schema and immutable Merkle-log store. Import
   RocksDB, PostgreSQL, contract and IPFS references as historical events with
   source, time, digest and trust-assumption labels.
3. Add identity/ownership events before enabling new package writes. Persist
   replay nonces and bind every submission to exact source, artifact, manifest
   and provenance digests.
4. Move asynchronous work to a transactional queue with idempotency keys,
   retry and dead-letter records. Rebuild PostgreSQL/search as derived views.
5. Add signed tree heads, independent witness interfaces, signed freshness
   metadata and client revocation checks before treating log decisions as
   install-eligible.
6. Replace legacy chain reads in CLI/API with canonical-log reads. Retain
   read-only compatibility for a declared window; do not roll back immutable
   history during database rollback.
7. Deploy a minimal checkpoint/bond contract only after reproducible tests and
   review. It checkpoints log roots and MUST NOT state package safety.

Rollback uses additive schemas, immutable event retention, derived-view
rebuilds and a feature-gated write cutover. It MUST NOT delete security events
or restore a superseded/revoked decision as current.
