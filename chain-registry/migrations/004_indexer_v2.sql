-- Preserve v1 block hashes as replay anchors and rebuild v2 evidence from genesis.
ALTER TABLE sync_state ALTER COLUMN last_height SET DEFAULT -1;
ALTER TABLE sync_state ADD COLUMN replay_through BIGINT NOT NULL DEFAULT -1;
UPDATE sync_state
SET last_height = -1, replay_through = COALESCE((SELECT max(height) FROM blocks), -1)
WHERE id = 1;

ALTER TABLE packages ADD COLUMN package_namespace TEXT;
UPDATE packages SET package_namespace = btrim(ecosystem) || ':' || btrim(name) || '@';
ALTER TABLE packages ALTER COLUMN package_namespace SET NOT NULL;
ALTER TABLE packages DROP CONSTRAINT IF EXISTS packages_status_check;
ALTER TABLE packages ADD CONSTRAINT packages_status_check
    CHECK (status IN ('pending', 'verified', 'quarantined', 'revoked'));
CREATE INDEX idx_packages_namespace_publisher ON packages (package_namespace, publisher_pubkey);

ALTER TABLE publisher_stats ALTER COLUMN stake_wei TYPE NUMERIC(78,0);
ALTER TABLE blocks
    ADD COLUMN validator_set_hash TEXT,
    ADD COLUMN view_number BIGINT CHECK (view_number BETWEEN 0 AND 4294967295),
    ADD COLUMN status_root TEXT,
    ADD COLUMN vote_v4_active BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN commit_signatures JSONB NOT NULL DEFAULT '[]',
    ADD COLUMN committee_epoch BIGINT NOT NULL DEFAULT 0;

-- An epoch identifies a contiguous validator-set hash run by its start height.
CREATE INDEX idx_blocks_committee_epoch ON blocks (committee_epoch, height);

CREATE TABLE package_status_history (
    canonical TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'verified', 'quarantined', 'revoked')),
    reason_code TEXT,
    reason TEXT,
    height BIGINT NOT NULL,
    tx_index INT NOT NULL,
    quarantine_expires_at TIMESTAMPTZ,
    source_tx_type TEXT NOT NULL,
    committee_summary JSONB,
    certificate_digest TEXT,
    PRIMARY KEY (canonical, height, tx_index)
);
CREATE INDEX idx_status_history_height ON package_status_history (height, tx_index)
    WHERE source_tx_type IN ('publish','publish_authorized_v1');

CREATE TABLE vote_signatures (
    canonical TEXT NOT NULL,
    height BIGINT NOT NULL,
    tx_index INT NOT NULL,
    validator_id TEXT NOT NULL,
    voter_key_id TEXT,
    scheme TEXT NOT NULL CHECK (scheme IN ('ed25519', 'hybrid_ed25519_mldsa44_v1')),
    approved BOOLEAN NOT NULL,
    reject_reason TEXT,
    evidence_digest TEXT NOT NULL,
    scanner_profile_digest TEXT,
    is_mandatory BOOLEAN NOT NULL,
    signed_at TIMESTAMPTZ NOT NULL,
    trigger_witness_digest TEXT,
    signature TEXT NOT NULL,
    pq_signature TEXT,
    PRIMARY KEY (canonical, height, tx_index, validator_id)
);

CREATE TABLE semantic_votes (
    canonical TEXT NOT NULL,
    height BIGINT NOT NULL,
    tx_index INT NOT NULL,
    validator_id TEXT NOT NULL,
    disposition TEXT NOT NULL CHECK (disposition IN ('approve', 'quarantine_review', 'reject')),
    model_family TEXT NOT NULL,
    confidence_bp SMALLINT CHECK (confidence_bp BETWEEN 0 AND 10000),
    finding_categories TEXT[] NOT NULL DEFAULT '{}',
    finding_digests TEXT[] NOT NULL DEFAULT '{}',
    PRIMARY KEY (canonical, height, tx_index, validator_id),
    FOREIGN KEY (canonical, height, tx_index, validator_id)
        REFERENCES vote_signatures (canonical, height, tx_index, validator_id)
);

CREATE TABLE trigger_witnesses (
    witness_digest TEXT PRIMARY KEY,
    canonical TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    manifest_sha256 TEXT NOT NULL,
    sandbox_policy_bundle_id TEXT NOT NULL,
    trigger_spec JSONB NOT NULL,
    observed_violations TEXT[] NOT NULL,
    graph_canonical_digest TEXT NOT NULL,
    prover_id TEXT NOT NULL,
    prover_key_id TEXT NOT NULL,
    hybrid BOOLEAN NOT NULL,
    height BIGINT NOT NULL,
    tx_index INT NOT NULL
);
CREATE INDEX idx_trigger_witnesses_canonical ON trigger_witnesses (canonical, height, tx_index);

CREATE TABLE revocations (
    canonical TEXT NOT NULL,
    height BIGINT NOT NULL,
    tx_index INT NOT NULL,
    tx_type TEXT NOT NULL CHECK (tx_type IN ('revoke', 'revoke_v2', 'quarantine_expiry', 'quarantine_resolution_v1')),
    authority TEXT NOT NULL CHECK (authority IN ('owner', 'controller', 'committee_quorum', 'legacy_single_validator', 'ttl_expiry')),
    reason TEXT,
    revoker_pubkey TEXT,
    evidence_hash TEXT,
    witness_digest TEXT REFERENCES trigger_witnesses(witness_digest),
    quorum_certificate JSONB,
    PRIMARY KEY (canonical, height, tx_index)
);

CREATE TABLE quarantines (
    canonical TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    opened_height BIGINT NOT NULL,
    opened_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    resolved_height BIGINT,
    resolution TEXT CHECK (resolution IN ('released', 'rejected', 'expired')),
    committee_summary JSONB NOT NULL
);
CREATE INDEX idx_quarantines_pending ON quarantines (expires_at, canonical) WHERE resolution IS NULL;
CREATE VIEW v_quarantine_queue AS
SELECT * FROM quarantines WHERE resolution IS NULL ORDER BY expires_at, canonical;

CREATE TABLE validator_events (
    height BIGINT NOT NULL,
    tx_index INT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('join', 'leave', 'slash')),
    validator_id TEXT NOT NULL,
    amount_wei NUMERIC(78,0),
    reason TEXT,
    PRIMARY KEY (height, tx_index)
);

CREATE TABLE discovery_bounties (
    l1_chain_id BIGINT NOT NULL,
    l1_block_number BIGINT NOT NULL,
    l1_block_hash TEXT NOT NULL,
    l1_tx_hash TEXT NOT NULL,
    log_index BIGINT NOT NULL,
    witness_digest TEXT NOT NULL,
    package_hash TEXT NOT NULL,
    prover_address TEXT NOT NULL,
    prover_bonus_wei NUMERIC(78,0) NOT NULL,
    committee_share_wei NUMERIC(78,0) NOT NULL,
    committee_share_status TEXT NOT NULL CHECK (committee_share_status IN ('reserved', 'distributed')),
    PRIMARY KEY (l1_chain_id, l1_tx_hash, log_index)
);
CREATE INDEX idx_discovery_bounties_witness ON discovery_bounties (witness_digest);
CREATE INDEX idx_discovery_bounties_prover ON discovery_bounties
    (l1_chain_id, prover_address, l1_block_number DESC, log_index DESC);
CREATE VIEW v_discovery_bounty_witnesses AS
SELECT bounty.*, witness.canonical, witness.height AS witness_height
FROM discovery_bounties bounty
LEFT JOIN trigger_witnesses witness USING (witness_digest);

CREATE TABLE l1_bounty_cursor (
    l1_chain_id BIGINT PRIMARY KEY,
    rewards_address TEXT NOT NULL,
    start_block BIGINT NOT NULL,
    last_block BIGINT,
    last_block_hash TEXT,
    CHECK ((last_block IS NULL) = (last_block_hash IS NULL))
);

CREATE TABLE namespace_controllers (
    package_namespace TEXT PRIMARY KEY,
    controller_pubkey TEXT NOT NULL,
    authorized BOOLEAN NOT NULL
);

CREATE TABLE index_events (
    height BIGINT NOT NULL,
    tx_index BIGINT NOT NULL CHECK (tx_index BETWEEN 0 AND 4294967295),
    envelope JSONB NOT NULL,
    PRIMARY KEY (height, tx_index)
);
