-- Initialize testnet database schema
-- Chain Registry Testnet PostgreSQL Schema

-- Enable UUID extension
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Packages table
CREATE TABLE IF NOT EXISTS packages (
    id SERIAL PRIMARY KEY,
    canonical TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    ecosystem TEXT NOT NULL,
    ipfs_cid TEXT NOT NULL,
    publisher_pubkey TEXT NOT NULL,
    block_hash TEXT NOT NULL,
    published_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes for packages
CREATE INDEX IF NOT EXISTS idx_packages_canonical ON packages(canonical);
CREATE INDEX IF NOT EXISTS idx_packages_name ON packages(name);
CREATE INDEX IF NOT EXISTS idx_packages_publisher ON packages(publisher_pubkey);
CREATE INDEX IF NOT EXISTS idx_packages_published_at ON packages(published_at);

-- Validator signatures/votes
CREATE TABLE IF NOT EXISTS validator_signatures (
    id SERIAL PRIMARY KEY,
    canonical TEXT NOT NULL,
    validator_id TEXT NOT NULL,
    validator_pubkey TEXT NOT NULL,
    signature TEXT NOT NULL,
    vote TEXT NOT NULL, -- 'approve', 'reject', 'abstain'
    reason TEXT,
    signed_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes for signatures
CREATE INDEX IF NOT EXISTS idx_sigs_canonical ON validator_signatures(canonical);
CREATE INDEX IF NOT EXISTS idx_sigs_validator ON validator_signatures(validator_id);
CREATE INDEX IF NOT EXISTS idx_sigs_signed_at ON validator_signatures(signed_at);

-- Chain blocks
CREATE TABLE IF NOT EXISTS chain_blocks (
    id SERIAL PRIMARY KEY,
    height BIGINT NOT NULL UNIQUE,
    block_hash TEXT NOT NULL UNIQUE,
    parent_hash TEXT NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL,
    proposer TEXT NOT NULL,
    tx_count INTEGER DEFAULT 0,
    data JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes for blocks
CREATE INDEX IF NOT EXISTS idx_blocks_height ON chain_blocks(height);
CREATE INDEX IF NOT EXISTS idx_blocks_hash ON chain_blocks(block_hash);
CREATE INDEX IF NOT EXISTS idx_blocks_proposer ON chain_blocks(proposer);

-- Pending transactions
CREATE TABLE IF NOT EXISTS pending_tx (
    id SERIAL PRIMARY KEY,
    tx_hash TEXT NOT NULL UNIQUE,
    tx_type TEXT NOT NULL, -- 'publish', 'revoke', 'stake', etc.
    sender TEXT NOT NULL,
    data JSONB NOT NULL,
    status TEXT DEFAULT 'pending', -- 'pending', 'included', 'failed'
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_pending_tx_status ON pending_tx(status);
CREATE INDEX IF NOT EXISTS idx_pending_tx_sender ON pending_tx(sender);

-- Faucet drips (for tracking)
CREATE TABLE IF NOT EXISTS faucet_drips (
    id SERIAL PRIMARY KEY,
    recipient TEXT NOT NULL,
    amount NUMERIC NOT NULL,
    tx_hash TEXT,
    ip_address INET,
    user_agent TEXT,
    dripped_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_faucet_recipient ON faucet_drips(recipient);
CREATE INDEX IF NOT EXISTS idx_faucet_dripped_at ON faucet_drips(dripped_at);

-- Testnet metrics
CREATE TABLE IF NOT EXISTS testnet_metrics (
    id SERIAL PRIMARY KEY,
    metric_name TEXT NOT NULL,
    metric_value NUMERIC NOT NULL,
    recorded_at TIMESTAMPTZ DEFAULT NOW()
);

-- Insert initial testnet marker
INSERT INTO testnet_metrics (metric_name, metric_value) 
VALUES ('testnet_initialized', 1)
ON CONFLICT DO NOTHING;

-- Create update trigger for updated_at
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

-- Apply triggers
DROP TRIGGER IF EXISTS update_packages_updated_at ON packages;
CREATE TRIGGER update_packages_updated_at 
    BEFORE UPDATE ON packages 
    FOR EACH ROW 
    EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_pending_tx_updated_at ON pending_tx;
CREATE TRIGGER update_pending_tx_updated_at 
    BEFORE UPDATE ON pending_tx 
    FOR EACH ROW 
    EXECUTE FUNCTION update_updated_at_column();

-- Comments for documentation
COMMENT ON TABLE packages IS 'Published packages on the testnet';
COMMENT ON TABLE validator_signatures IS 'Validator votes on packages';
COMMENT ON TABLE chain_blocks IS 'Chain blocks for explorer';
COMMENT ON TABLE faucet_drips IS 'Testnet faucet distribution log';
