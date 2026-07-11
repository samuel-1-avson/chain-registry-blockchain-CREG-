# ADR-002: Canonical event history and source of truth

- **Context/problem:** Rust chain, RocksDB, PostgreSQL, contracts, and APIs can disagree.
- **Options:** contract state; local chain; witnessed append-only log.
- **Decision:** the witnessed event log is canonical; all databases and UIs are rebuildable views.
- **Security implications:** inclusion/consistency proofs and tombstones prevent silent rewrite.
- **Operational implications:** replay/rebuild tooling and retention manifests are mandatory.
- **Migration implications:** import legacy records with explicit limitations.
- **Rejected alternatives:** treating an indexer or explorer as authoritative.
- **Revisit conditions:** an independently audited appchain proves a measurable need.
