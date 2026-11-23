# PostgreSQL migrations (`db-sync`)

Canonical schema for the indexer / `db-sync` service. Apply in order:

1. `001_db_sync_bootstrap.sql` — core tables
2. `002_testnet_extras.sql` — testnet-only extensions
3. `003_stake_wei_numeric.sql` — widens `publisher_stats.stake_wei` to `NUMERIC(78,0)` (ECA-23); idempotent, also applied by the `db-sync` worker on startup

Legacy bootstrap SQL in `testnet/init-testnet-db.sql` is deprecated; new environments should use these files only.
