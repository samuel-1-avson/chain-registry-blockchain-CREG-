BEGIN;

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS arena_actors (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    hub_subject TEXT UNIQUE,
    display_name TEXT NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 80),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'deleted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS arena_identities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id UUID NOT NULL REFERENCES arena_actors(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK (provider IN ('github', 'ethereum', 'creg', 'email')),
    provider_subject TEXT NOT NULL,
    display_value TEXT,
    verified_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    UNIQUE (provider, provider_subject)
);

CREATE TABLE IF NOT EXISTS arena_contributions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id UUID NOT NULL REFERENCES arena_actors(id),
    kind TEXT NOT NULL CHECK (kind IN (
        'documentation', 'bug', 'security', 'code', 'test', 'ux',
        'accessibility', 'validator_ops', 'publisher_ops', 'governance', 'research'
    )),
    visibility TEXT NOT NULL DEFAULT 'public' CHECK (visibility IN ('public', 'private_security')),
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN (
        'draft', 'submitted', 'triage', 'community_review', 'private_security_review',
        'maintainer_review', 'accepted', 'rejected', 'implemented', 'verified', 'withdrawn'
    )),
    title TEXT NOT NULL CHECK (char_length(title) BETWEEN 8 AND 180),
    description TEXT NOT NULL CHECK (char_length(description) BETWEEN 40 AND 10000),
    target_type TEXT NOT NULL CHECK (target_type IN (
        'repository', 'issue', 'pull_request', 'commit', 'document', 'package',
        'transaction', 'validator', 'network', 'incident', 'governance_proposal'
    )),
    target_ref TEXT NOT NULL CHECK (char_length(target_ref) BETWEEN 1 AND 1000),
    source_revision TEXT,
    submitted_at TIMESTAMPTZ,
    resolved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS arena_evidence (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contribution_id UUID NOT NULL REFERENCES arena_contributions(id) ON DELETE CASCADE,
    evidence_type TEXT NOT NULL CHECK (evidence_type IN (
        'source_reference', 'reproduction', 'test', 'log', 'screenshot', 'issue',
        'pull_request', 'commit', 'transaction', 'package_record', 'incident', 'other'
    )),
    reference TEXT NOT NULL CHECK (char_length(reference) BETWEEN 1 AND 2000),
    content_hash TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS arena_reviews (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contribution_id UUID NOT NULL REFERENCES arena_contributions(id) ON DELETE CASCADE,
    reviewer_id UUID NOT NULL REFERENCES arena_actors(id),
    decision TEXT NOT NULL CHECK (decision IN (
        'needs_information', 'request_changes', 'recommend_accept', 'accept', 'reject',
        'implemented', 'verified'
    )),
    note TEXT NOT NULL CHECK (char_length(note) BETWEEN 1 AND 4000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS arena_votes (
    contribution_id UUID NOT NULL REFERENCES arena_contributions(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES arena_actors(id) ON DELETE CASCADE,
    vote SMALLINT NOT NULL CHECK (vote IN (-1, 1)),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (contribution_id, actor_id)
);

CREATE TABLE IF NOT EXISTS arena_comments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contribution_id UUID NOT NULL REFERENCES arena_contributions(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES arena_actors(id),
    body TEXT NOT NULL CHECK (char_length(body) BETWEEN 2 AND 2000),
    moderation_status TEXT NOT NULL DEFAULT 'visible' CHECK (moderation_status IN ('visible', 'hidden', 'removed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS arena_reputation_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_id UUID NOT NULL REFERENCES arena_actors(id),
    contribution_id UUID REFERENCES arena_contributions(id),
    event_type TEXT NOT NULL CHECK (event_type IN (
        'accepted', 'implemented', 'verified', 'review', 'moderation_adjustment', 'reversal'
    )),
    points INTEGER NOT NULL CHECK (points BETWEEN -1000 AND 1000),
    policy_version TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (char_length(reason) BETWEEN 1 AND 1000),
    awarded_by UUID REFERENCES arena_actors(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS arena_reputation_once_per_outcome
    ON arena_reputation_events (actor_id, contribution_id, event_type, policy_version)
    WHERE contribution_id IS NOT NULL AND event_type IN ('accepted', 'implemented', 'verified');

CREATE TABLE IF NOT EXISTS arena_audit_events (
    id BIGSERIAL PRIMARY KEY,
    actor_id UUID REFERENCES arena_actors(id),
    contribution_id UUID REFERENCES arena_contributions(id),
    action TEXT NOT NULL,
    previous_state JSONB,
    next_state JSONB,
    request_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS arena_contributions_actor_idx ON arena_contributions (actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS arena_contributions_status_idx ON arena_contributions (status, created_at DESC);
CREATE INDEX IF NOT EXISTS arena_contributions_target_idx ON arena_contributions (target_type, target_ref);
CREATE INDEX IF NOT EXISTS arena_evidence_contribution_idx ON arena_evidence (contribution_id);
CREATE INDEX IF NOT EXISTS arena_reviews_contribution_idx ON arena_reviews (contribution_id, created_at DESC);
CREATE INDEX IF NOT EXISTS arena_comments_contribution_idx ON arena_comments (contribution_id, created_at);
CREATE INDEX IF NOT EXISTS arena_reputation_actor_idx ON arena_reputation_events (actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS arena_audit_contribution_idx ON arena_audit_events (contribution_id, created_at DESC);

COMMIT;
