//! PostgreSQL schema definitions for the chain registry mirror.
//!
//! Run these statements once during node initialisation (e.g. via `sqlx migrate`).

/// Full schema bootstrap SQL.
pub const INIT_SQL: &str = r#"
-- Sync cursor tracking
CREATE TABLE IF NOT EXISTS sync_state (
    id              INT PRIMARY KEY DEFAULT 1,
    last_height     BIGINT NOT NULL DEFAULT 0,
    updated_at      TIMESTAMPTZ DEFAULT NOW()
);

INSERT INTO sync_state (id, last_height) VALUES (1, 0)
ON CONFLICT (id) DO NOTHING;

-- Package records (mirrored from sled)
CREATE TABLE IF NOT EXISTS packages (
    canonical        TEXT PRIMARY KEY,
    ecosystem        TEXT NOT NULL,
    name             TEXT NOT NULL,
    version          TEXT NOT NULL,
    status           TEXT NOT NULL CHECK (status IN ('verified', 'pending', 'quarantined', 'revoked')),
    content_hash     TEXT NOT NULL,
    ipfs_cid         TEXT NOT NULL,
    publisher_pubkey TEXT NOT NULL,
    block_hash       TEXT NOT NULL,
    published_at     TIMESTAMPTZ NOT NULL,
    shielded         BOOLEAN DEFAULT FALSE,
    findings         JSONB DEFAULT '[]',
    access_count     INT DEFAULT 0,
    last_accessed    TIMESTAMPTZ,
    revocation_reason TEXT,
    created_at       TIMESTAMPTZ DEFAULT NOW(),
    updated_at       TIMESTAMPTZ DEFAULT NOW()
);

ALTER TABLE packages ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'verified';
ALTER TABLE packages ADD COLUMN IF NOT EXISTS content_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE packages ADD COLUMN IF NOT EXISTS shielded BOOLEAN DEFAULT FALSE;
ALTER TABLE packages ADD COLUMN IF NOT EXISTS findings JSONB DEFAULT '[]';
ALTER TABLE packages ADD COLUMN IF NOT EXISTS access_count INT DEFAULT 0;
ALTER TABLE packages ADD COLUMN IF NOT EXISTS last_accessed TIMESTAMPTZ;
ALTER TABLE packages ADD COLUMN IF NOT EXISTS revocation_reason TEXT;

UPDATE packages SET status = 'verified' WHERE status IS NULL;
UPDATE packages SET content_hash = '' WHERE content_hash IS NULL;
UPDATE packages SET shielded = FALSE WHERE shielded IS NULL;
UPDATE packages SET findings = '[]'::jsonb WHERE findings IS NULL;
UPDATE packages SET access_count = 0 WHERE access_count IS NULL;

CREATE INDEX IF NOT EXISTS idx_packages_ecosystem ON packages(ecosystem);
CREATE INDEX IF NOT EXISTS idx_packages_publisher  ON packages(publisher_pubkey);
CREATE INDEX IF NOT EXISTS idx_packages_status     ON packages(status);
CREATE INDEX IF NOT EXISTS idx_packages_name       ON packages(name);

-- Validator votes per package
CREATE TABLE IF NOT EXISTS validator_votes (
    id               BIGSERIAL PRIMARY KEY,
    canonical        TEXT NOT NULL,
    validator_id     TEXT NOT NULL,
    validator_pubkey TEXT NOT NULL,
    signature        TEXT NOT NULL,
    vote             TEXT NOT NULL CHECK (vote IN ('approve', 'reject')),
    reason           TEXT,
    signed_at        TIMESTAMPTZ NOT NULL,
    created_at       TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (canonical, validator_id)
);

CREATE INDEX IF NOT EXISTS idx_votes_canonical    ON validator_votes(canonical);
CREATE INDEX IF NOT EXISTS idx_votes_validator    ON validator_votes(validator_id);

-- Block headers for explorer
CREATE TABLE IF NOT EXISTS blocks (
    height           BIGINT PRIMARY KEY,
    hash             TEXT NOT NULL UNIQUE,
    prev_hash        TEXT NOT NULL,
    merkle_root      TEXT NOT NULL,
    proposer_id      TEXT NOT NULL,
    timestamp        TIMESTAMPTZ NOT NULL,
    created_at       TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_blocks_hash ON blocks(hash);

-- Aggregated publisher stats (rebuilt on demand or incrementally)
CREATE TABLE IF NOT EXISTS publisher_stats (
    pubkey           TEXT PRIMARY KEY,
    total_packages   INT DEFAULT 0,
    verified_count   INT DEFAULT 0,
    revoked_count    INT DEFAULT 0,
    stake_wei        NUMERIC(78,0) DEFAULT 0,
    first_seen_at    TIMESTAMPTZ,
    first_seen_days  INT DEFAULT 0,
    updated_at       TIMESTAMPTZ DEFAULT NOW()
);
"#;

/// ECA-23 (ADR-019 Increment 1) hot-fix: wei amounts are `uint256`, so they
/// need `NUMERIC(78,0)`. `BIGINT` overflows above ~9.22 tokens at 18
/// decimals (the validator minimum is 100). Idempotent: rewrites the column
/// only while it is not already `numeric`, so it is safe on every startup.
/// Mirrored by `migrations/003_stake_wei_numeric.sql`.
pub const STAKE_WEI_NUMERIC_SQL: &str = r#"
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
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wei_columns_are_numeric_78() {
        assert!(!INIT_SQL.contains("stake_wei        BIGINT"));
        assert!(INIT_SQL.contains("stake_wei        NUMERIC(78,0)"));
        assert!(STAKE_WEI_NUMERIC_SQL.contains("TYPE NUMERIC(78,0)"));
        // Compare without CR so `core.autocrlf` checkouts agree with LF ones.
        let migration =
            include_str!("../../../migrations/003_stake_wei_numeric.sql").replace('\r', "");
        let body = migration
            .split_once("-- BEGIN STAKE_WEI_NUMERIC_SQL")
            .and_then(|(_, rest)| rest.split_once("-- END STAKE_WEI_NUMERIC_SQL"))
            .map(|(body, _)| body.to_string())
            .unwrap_or_default();
        assert_eq!(
            STAKE_WEI_NUMERIC_SQL.replace('\r', ""),
            body,
            "migration file and embedded hot-fix drifted"
        );
    }
}
