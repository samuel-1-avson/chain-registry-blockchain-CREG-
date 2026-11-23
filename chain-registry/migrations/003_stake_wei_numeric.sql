-- 003_stake_wei_numeric.sql
-- ECA-23 (ADR-019 Increment 1): publisher_stats.stake_wei BIGINT overflows
-- above ~9.22 tokens at 18 decimals. Wei amounts are uint256 and need
-- NUMERIC(78,0). Idempotent; the db-sync worker also applies it on startup
-- (crates/db-sync/src/schema.rs STAKE_WEI_NUMERIC_SQL, kept byte-identical by
-- a unit test).
-- BEGIN STAKE_WEI_NUMERIC_SQL
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'publisher_stats'
          AND column_name = 'stake_wei'
          AND data_type <> 'numeric'
    ) THEN
        ALTER TABLE publisher_stats ALTER COLUMN stake_wei TYPE NUMERIC(78,0);
    END IF;
END $$;
-- END STAKE_WEI_NUMERIC_SQL
