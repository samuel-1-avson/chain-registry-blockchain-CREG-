import { randomUUID } from "node:crypto";
import type Database from "better-sqlite3";
import { normalizeAddress } from "../db/index.js";
import type {
  ArenaActor,
  ArenaContribution,
  ArenaRole,
  ContributionStatus,
} from "./domain.js";
import type {
  AuditRecord,
  ContributionRepository,
  CreateContributionRecord,
  ReputationRecord,
} from "./repository.js";

type ContributionRow = {
  id: string;
  actor_id: string;
  kind: ArenaContribution["kind"];
  visibility: ArenaContribution["visibility"];
  status: ContributionStatus;
  title: string;
  description: string;
  target_type: ArenaContribution["targetType"];
  target_ref: string;
  source_revision: string | null;
  created_at: string;
  updated_at: string;
};

function nowIso(): string {
  return new Date().toISOString();
}

function mapContribution(row: ContributionRow): ArenaContribution {
  return {
    id: row.id,
    actorId: row.actor_id,
    kind: row.kind,
    visibility: row.visibility,
    status: row.status,
    title: row.title,
    description: row.description,
    targetType: row.target_type,
    targetRef: row.target_ref,
    ...(row.source_revision ? { sourceRevision: row.source_revision } : {}),
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  };
}

function parseAddressList(value: string | undefined): Set<string> {
  return new Set(
    (value ?? "")
      .split(",")
      .map((entry) => entry.trim().toLowerCase())
      .filter((entry) => /^0x[a-f0-9]{40}$/.test(entry)),
  );
}

export function roleForAddress(address: string): ArenaRole {
  const normalized = normalizeAddress(address);
  const maintainers = parseAddressList(process.env.HUB_ARENA_MAINTAINER_ADDRESSES);
  const reviewers = parseAddressList(process.env.HUB_ARENA_REVIEWER_ADDRESSES);
  const admins = parseAddressList(process.env.HUB_ARENA_ADMIN_ADDRESSES);
  if (admins.has(normalized)) return "admin";
  if (maintainers.has(normalized)) return "maintainer";
  if (reviewers.has(normalized)) return "reviewer";
  return "contributor";
}

export class SqliteContributionRepository implements ContributionRepository {
  constructor(private readonly db: Database.Database) {}

  async withTransaction<T>(
    work: (repository: ContributionRepository) => Promise<T>,
  ): Promise<T> {
    // Repository methods are synchronous under an async facade. Nested
    // better-sqlite3 transactions would require a fully sync API; sequential
    // statements remain atomic individually for this foundation slice.
    return work(this);
  }

  async createContribution(input: CreateContributionRecord): Promise<ArenaContribution> {
    const id = randomUUID();
    const createdAt = nowIso();
    this.db
      .prepare(
        `INSERT INTO arena_contributions (
          id, actor_id, kind, visibility, status, title, description,
          target_type, target_ref, source_revision, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        id,
        input.actorId,
        input.kind,
        input.visibility,
        input.status,
        input.title,
        input.description,
        input.targetType,
        input.targetRef,
        input.sourceRevision ?? null,
        createdAt,
        createdAt,
      );

    const created = await this.findContributionById(id, { includePrivate: true });
    if (!created) throw new Error("Failed to load contribution after insert.");
    return created;
  }

  async findContributionById(
    id: string,
    options?: { includePrivate?: boolean },
  ): Promise<ArenaContribution | null> {
    const row = this.db
      .prepare(
        `SELECT id, actor_id, kind, visibility, status, title, description,
                target_type, target_ref, source_revision, created_at, updated_at
         FROM arena_contributions WHERE id = ?`,
      )
      .get(id) as ContributionRow | undefined;
    if (!row) return null;
    if (!options?.includePrivate && row.visibility === "private_security") {
      return null;
    }
    return mapContribution(row);
  }

  async updateContributionStatus(
    id: string,
    expectedStatus: ContributionStatus,
    nextStatus: ContributionStatus,
  ): Promise<ArenaContribution | null> {
    const updatedAt = nowIso();
    const result = this.db
      .prepare(
        `UPDATE arena_contributions
         SET status = ?, updated_at = ?,
             submitted_at = CASE WHEN ? = 'submitted' THEN ? ELSE submitted_at END,
             resolved_at = CASE WHEN ? IN ('accepted', 'rejected', 'verified', 'withdrawn') THEN ? ELSE resolved_at END
         WHERE id = ? AND status = ?`,
      )
      .run(nextStatus, updatedAt, nextStatus, updatedAt, nextStatus, updatedAt, id, expectedStatus);
    if (result.changes === 0) return null;
    return this.findContributionById(id, { includePrivate: true });
  }

  async upsertVote(input: {
    contributionId: string;
    actorId: string;
    vote: -1 | 1;
  }): Promise<"created" | "updated" | "unchanged"> {
    const existing = this.db
      .prepare(
        `SELECT vote FROM arena_votes WHERE contribution_id = ? AND actor_id = ?`,
      )
      .get(input.contributionId, input.actorId) as { vote: number } | undefined;

    if (!existing) {
      const createdAt = nowIso();
      this.db
        .prepare(
          `INSERT INTO arena_votes (contribution_id, actor_id, vote, created_at, updated_at)
           VALUES (?, ?, ?, ?, ?)`,
        )
        .run(input.contributionId, input.actorId, input.vote, createdAt, createdAt);
      return "created";
    }

    if (existing.vote === input.vote) return "unchanged";

    this.db
      .prepare(
        `UPDATE arena_votes SET vote = ?, updated_at = ? WHERE contribution_id = ? AND actor_id = ?`,
      )
      .run(input.vote, nowIso(), input.contributionId, input.actorId);
    return "updated";
  }

  async appendAudit(record: AuditRecord): Promise<void> {
    this.db
      .prepare(
        `INSERT INTO arena_audit_events (
          actor_id, contribution_id, action, previous_state, next_state, request_id, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        record.actorId,
        record.contributionId,
        record.action,
        record.previousState === undefined ? null : JSON.stringify(record.previousState),
        record.nextState === undefined ? null : JSON.stringify(record.nextState),
        record.requestId ?? null,
        nowIso(),
      );
  }

  async appendReputation(record: ReputationRecord): Promise<"created" | "duplicate"> {
    try {
      this.db
        .prepare(
          `INSERT INTO arena_reputation_events (
            id, actor_id, contribution_id, event_type, points, policy_version, reason, awarded_by, created_at
          ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
        )
        .run(
          randomUUID(),
          record.actorId,
          record.contributionId,
          record.eventType,
          record.points,
          record.policyVersion,
          record.reason,
          record.awardedBy,
          nowIso(),
        );
      return "created";
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("UNIQUE") || message.includes("constraint")) {
        return "duplicate";
      }
      throw error;
    }
  }

  ensureActorForEthereumAddress(address: string): ArenaActor {
    const normalized = normalizeAddress(address);
    const existing = this.db
      .prepare(
        `SELECT a.id
         FROM arena_identities i
         JOIN arena_actors a ON a.id = i.actor_id
         WHERE i.provider = 'ethereum' AND i.provider_subject = ? AND i.revoked_at IS NULL`,
      )
      .get(normalized) as { id: string } | undefined;

    const role = roleForAddress(normalized);
    if (existing) {
      return { id: existing.id, role, identityVerified: true };
    }

    const actorId = randomUUID();
    const identityId = randomUUID();
    const createdAt = nowIso();
    const displayName = `${normalized.slice(0, 6)}…${normalized.slice(-4)}`;

    const insert = this.db.transaction(() => {
      this.db
        .prepare(
          `INSERT INTO arena_actors (id, hub_subject, display_name, status, created_at, updated_at)
           VALUES (?, ?, ?, 'active', ?, ?)`,
        )
        .run(actorId, `eth:${normalized}`, displayName, createdAt, createdAt);
      this.db
        .prepare(
          `INSERT INTO arena_identities (
            id, actor_id, provider, provider_subject, display_value, verified_at, metadata
          ) VALUES (?, ?, 'ethereum', ?, ?, ?, '{}')`,
        )
        .run(identityId, actorId, normalized, normalized, createdAt);
    });
    insert();

    return { id: actorId, role, identityVerified: true };
  }
}
