-- Contribution Arena foundation tables (SQLite).
-- Hub is not the live Arena authority until migration gates in
-- CREG-Contribution-Arena/docs/AUTHORITY_DECISION.md are complete.

CREATE TABLE IF NOT EXISTS arena_actors (
    id TEXT PRIMARY KEY NOT NULL,
    hub_subject TEXT UNIQUE,
    display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 80),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'deleted')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS arena_identities (
    id TEXT PRIMARY KEY NOT NULL,
    actor_id TEXT NOT NULL REFERENCES arena_actors(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK (provider IN ('github', 'ethereum', 'creg', 'email')),
    provider_subject TEXT NOT NULL,
    display_value TEXT,
    verified_at TEXT NOT NULL,
    revoked_at TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    UNIQUE (provider, provider_subject)
);

CREATE TABLE IF NOT EXISTS arena_contributions (
    id TEXT PRIMARY KEY NOT NULL,
    actor_id TEXT NOT NULL REFERENCES arena_actors(id),
    kind TEXT NOT NULL CHECK (kind IN (
        'documentation', 'bug', 'security', 'code', 'test', 'ux',
        'accessibility', 'validator_ops', 'publisher_ops', 'governance', 'research'
    )),
    visibility TEXT NOT NULL DEFAULT 'public' CHECK (visibility IN ('public', 'private_security')),
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN (
        'draft', 'submitted', 'triage', 'community_review', 'private_security_review',
        'maintainer_review', 'accepted', 'rejected', 'implemented', 'verified', 'withdrawn'
    )),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 8 AND 180),
    description TEXT NOT NULL CHECK (length(description) BETWEEN 40 AND 10000),
    target_type TEXT NOT NULL CHECK (target_type IN (
        'repository', 'issue', 'pull_request', 'commit', 'document', 'package',
        'transaction', 'validator', 'network', 'incident', 'governance_proposal'
    )),
    target_ref TEXT NOT NULL CHECK (length(target_ref) BETWEEN 1 AND 1000),
    source_revision TEXT,
    submitted_at TEXT,
    resolved_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS arena_evidence (
    id TEXT PRIMARY KEY NOT NULL,
    contribution_id TEXT NOT NULL REFERENCES arena_contributions(id) ON DELETE CASCADE,
    evidence_type TEXT NOT NULL CHECK (evidence_type IN (
        'source_reference', 'reproduction', 'test', 'log', 'screenshot', 'issue',
        'pull_request', 'commit', 'transaction', 'package_record', 'incident', 'other'
    )),
    reference TEXT NOT NULL CHECK (length(reference) BETWEEN 1 AND 2000),
    content_hash TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS arena_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    contribution_id TEXT NOT NULL REFERENCES arena_contributions(id) ON DELETE CASCADE,
    reviewer_id TEXT NOT NULL REFERENCES arena_actors(id),
    decision TEXT NOT NULL CHECK (decision IN (
        'needs_information', 'request_changes', 'recommend_accept', 'accept', 'reject',
        'implemented', 'verified'
    )),
    note TEXT NOT NULL CHECK (length(note) BETWEEN 1 AND 4000),
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS arena_votes (
    contribution_id TEXT NOT NULL REFERENCES arena_contributions(id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL REFERENCES arena_actors(id) ON DELETE CASCADE,
    vote INTEGER NOT NULL CHECK (vote IN (-1, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (contribution_id, actor_id)
);

CREATE TABLE IF NOT EXISTS arena_comments (
    id TEXT PRIMARY KEY NOT NULL,
    contribution_id TEXT NOT NULL REFERENCES arena_contributions(id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL REFERENCES arena_actors(id),
    body TEXT NOT NULL CHECK (length(body) BETWEEN 2 AND 2000),
    moderation_status TEXT NOT NULL DEFAULT 'visible' CHECK (moderation_status IN ('visible', 'hidden', 'removed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS arena_reputation_events (
    id TEXT PRIMARY KEY NOT NULL,
    actor_id TEXT NOT NULL REFERENCES arena_actors(id),
    contribution_id TEXT REFERENCES arena_contributions(id),
    event_type TEXT NOT NULL CHECK (event_type IN (
        'accepted', 'implemented', 'verified', 'review', 'moderation_adjustment', 'reversal'
    )),
    points INTEGER NOT NULL CHECK (points BETWEEN -1000 AND 1000),
    policy_version TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 1000),
    awarded_by TEXT REFERENCES arena_actors(id),
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS arena_reputation_once_per_outcome
    ON arena_reputation_events (actor_id, contribution_id, event_type, policy_version)
    WHERE contribution_id IS NOT NULL AND event_type IN ('accepted', 'implemented', 'verified');

CREATE TABLE IF NOT EXISTS arena_audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_id TEXT REFERENCES arena_actors(id),
    contribution_id TEXT REFERENCES arena_contributions(id),
    action TEXT NOT NULL,
    previous_state TEXT,
    next_state TEXT,
    request_id TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS arena_contributions_actor_idx ON arena_contributions (actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS arena_contributions_status_idx ON arena_contributions (status, created_at DESC);
CREATE INDEX IF NOT EXISTS arena_contributions_target_idx ON arena_contributions (target_type, target_ref);
CREATE INDEX IF NOT EXISTS arena_evidence_contribution_idx ON arena_evidence (contribution_id);
CREATE INDEX IF NOT EXISTS arena_reviews_contribution_idx ON arena_reviews (contribution_id, created_at DESC);
CREATE INDEX IF NOT EXISTS arena_comments_contribution_idx ON arena_comments (contribution_id, created_at);
CREATE INDEX IF NOT EXISTS arena_reputation_actor_idx ON arena_reputation_events (actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS arena_audit_contribution_idx ON arena_audit_events (contribution_id, created_at DESC);
