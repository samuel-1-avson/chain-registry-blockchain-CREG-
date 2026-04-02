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
    status           TEXT NOT NULL CHECK (status IN ('verified', 'pending', 'revoked')),
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
    stake_wei        BIGINT DEFAULT 0,
    first_seen_at    TIMESTAMPTZ,
    first_seen_days  INT DEFAULT 0,
    updated_at       TIMESTAMPTZ DEFAULT NOW()
);
"#;
