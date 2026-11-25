# Database Schema

The PostgreSQL mirror schema lives in [the indexer migrations](../chain-registry/migrations/README.md). The standalone `creg-indexer` applies the embedded SQLx migrations when it starts. Consensus and the node's chain reads use RocksDB.

| Table or view | Purpose |
|---|---|
| `sync_state`, `blocks` | Canonical ingest cursor, block commitments and PBFT commit signatures |
| `packages`, `validator_votes`, `publisher_stats` | Package mirror, legacy validator votes and publisher metrics |
| `package_status_history`, `vote_signatures`, `semantic_votes` | Package transitions and vote evidence by block and transaction |
| `trigger_witnesses`, `revocations`, `quarantines`, `v_quarantine_queue` | Witness and revocation records and the active Class C queue |
| `validator_events` | Validator joins, leaves and slashes |
| `l1_bounty_cursor`, `discovery_bounties`, `v_discovery_bounty_witnesses` | L1 progress, confirmed bounty logs and their witness associations |
| `index_events` | Ordered SSE v2 replay records |

Stake and bounty amounts use `NUMERIC(78,0)`. Public bounty amounts are decimal strings.
