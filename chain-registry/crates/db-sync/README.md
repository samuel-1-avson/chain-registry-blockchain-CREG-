# db-sync — PostgreSQL Mirror Worker

Mirrors the sled-backed blockchain into PostgreSQL for fast queries, search, and analytics.

## Quick Start

1. Ensure PostgreSQL 14+ is running.
2. Set `CREG_PG_URL` (defaults to `postgres://localhost/chain_registry`).
3. The sync worker auto-creates tables on startup.

## Schema

See `src/schema.rs` for the full DDL. Key tables:

- `packages` — canonical package records with JSONB findings
- `validator_votes` — per-package consensus signatures
- `blocks` — block headers for the explorer
- `publisher_stats` — aggregated publisher metrics
- `sync_state` — cursor tracking (`last_height`)

## Integration

In the `node` crate, spawn the worker after chain initialisation:

```rust
let chain_proxy = Arc::new(RwLock::new(chain_store));
let sync_worker = db_sync::SyncWorker::new(
    db_sync::SyncConfig::default(),
    chain_proxy,
).await?;
tokio::spawn(sync_worker.run());
```

## Backfill

If the mirror falls behind, the worker automatically catches up block-by-block. To force a full rebuild, truncate `sync_state` and restart the worker.
