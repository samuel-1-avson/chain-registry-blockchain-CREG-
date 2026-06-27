import type {
  ArenaContribution,
  ContributionStatus,
  CreateContributionInput,
} from "./domain";

export interface CreateContributionRecord extends CreateContributionInput {
  actorId: string;
  visibility: "public" | "private_security";
  status: "draft";
}

export interface AuditRecord {
  actorId: string;
  contributionId: string;
  action: string;
  previousState?: unknown;
  nextState?: unknown;
  requestId?: string;
}

export interface ReputationRecord {
  actorId: string;
  contributionId: string;
  eventType: "accepted" | "implemented" | "verified";
  points: number;
  policyVersion: string;
  reason: string;
  awardedBy: string;
}

export interface ContributionRepository {
  withTransaction<T>(work: (repository: ContributionRepository) => Promise<T>): Promise<T>;
  createContribution(input: CreateContributionRecord): Promise<ArenaContribution>;
  findContributionById(id: string, options?: { includePrivate?: boolean }): Promise<ArenaContribution | null>;
  updateContributionStatus(id: string, expectedStatus: ContributionStatus, nextStatus: ContributionStatus): Promise<ArenaContribution | null>;
  upsertVote(input: { contributionId: string; actorId: string; vote: -1 | 1 }): Promise<"created" | "updated" | "unchanged">;
  appendAudit(record: AuditRecord): Promise<void>;
  appendReputation(record: ReputationRecord): Promise<"created" | "duplicate">;
}
