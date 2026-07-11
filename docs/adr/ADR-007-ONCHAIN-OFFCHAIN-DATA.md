# ADR-007: On-chain versus off-chain data

- **Context/problem:** package bytes, traces, and private data are too large/sensitive for consensus state.
- **Options:** all on-chain; all central; content-addressed off-chain objects plus compact commitments.
- **Decision:** keep bytes/evidence off-chain; commit typed digests, state events, witness roots, and bonds.
- **Security implications:** digest binding and availability proofs are distinct controls.
- **Operational implications:** replication, legal retention, and retrieval probes are required.
- **Migration implications:** convert existing records to subjects and evidence references.
- **Rejected alternatives:** treating IPFS as automatic permanence.
- **Revisit conditions:** regulatory or availability requirements change.
