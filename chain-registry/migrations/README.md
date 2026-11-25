# PostgreSQL migrations

The standalone `creg-indexer` embeds these migrations and runs them on startup with `sqlx::migrate!`. SQLx tracks applied versions and checksums in `_sqlx_migrations`. Append migrations for schema changes so that existing databases keep their migration history.

1. `001_db_sync_bootstrap.sql` creates the legacy mirror tables.
2. `002_testnet_extras.sql` creates the shared testnet extras.
3. `003_stake_wei_numeric.sql` widens publisher stake to `NUMERIC(78,0)`.
4. `004_indexer_v2.sql` adds v4 evidence, the L1 bounty lane and SSE replay.

Stop the v1 writer before upgrading. Upgrading a v1 mirror replays the chain from genesis to populate the new projection. Existing block hashes act as replay anchors. The index API returns 503 until the replay reaches the previous indexed height. A continuity or hash error pauses ingestion and surfaces through `/health` and the service log. The historical testnet bootstrap SQL is retained for legacy installations.

## Start the indexer

Set `CREG_PG_URL` and run the `creg-indexer` binary from the standalone package. The service defaults to `127.0.0.1:8084`. Override it with `CREG_INDEXER_LISTEN` and set the polling interval in seconds with `CREG_INDEXER_POLL_INTERVAL_SECS`.

- Local RocksDB mode uses `CREG_INDEXER_DATA_DIR` or `CREG_DATA_DIR` for the primary. Set `CREG_INDEXER_SECONDARY_DIR` to a writable directory separate from `chain.rocksdb` when running multiple instances on one host.
- Remote mode uses `CREG_INDEXER_REMOTE_URL`. It also needs the resolver's build-time `CREG_SPEC_SIGNING_PUBKEY` and a signed spec configured with `CREG_CHAIN_SPEC_PATH` or `CREG_CHAIN_SPEC_URL`. The upstream must serve the public blocks and validator-set-transition routes.
- L1 bounty ingest is enabled by `CREG_INDEXER_REWARDS_ADDRESS`. Set `CREG_INDEXER_L1_CHAIN_ID`, `CREG_INDEXER_L1_START_BLOCK` to the contract deployment height and `CREG_INDEXER_L1_RPC_URL`. Add independent RPC providers in the comma-separated `CREG_INDEXER_L1_RPC_FALLBACK_URLS` for multi-provider quorum reads. The PostgreSQL cursor binds the selected chain ID to the contract and start height.

The committee diversity endpoint uses the first height of a contiguous validator-set run as its epoch identifier. Epoch 0 covers the bootstrap set. Evidence history supports `before=<height.tx_index>` and SSE resumes with `Last-Event-ID` or `after=<height.tx_index>`.
