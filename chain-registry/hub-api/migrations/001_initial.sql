-- Hub API v1 schema (Phase 2 prep). All app queries must use bound parameters.

CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY NOT NULL,
  address TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_address ON sessions (address);
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions (expires_at);

CREATE TABLE IF NOT EXISTS nonces (
  nonce TEXT PRIMARY KEY NOT NULL,
  address TEXT NOT NULL,
  issued_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  consumed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_nonces_address ON nonces (address);
CREATE INDEX IF NOT EXISTS idx_nonces_expires_at ON nonces (expires_at);

CREATE TABLE IF NOT EXISTS quest_progress (
  address TEXT NOT NULL,
  quest_id TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('locked', 'available', 'in_progress', 'completed')),
  updated_at TEXT NOT NULL,
  PRIMARY KEY (address, quest_id)
);

CREATE INDEX IF NOT EXISTS idx_quest_progress_address ON quest_progress (address);
