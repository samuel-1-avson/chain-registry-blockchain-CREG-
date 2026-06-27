export type ArenaRole =
  | "guest"
  | "contributor"
  | "reviewer"
  | "security_reviewer"
  | "maintainer"
  | "moderator"
  | "admin";

export type ContributionVisibility = "public" | "private_security";

export type ContributionStatus =
  | "draft"
  | "submitted"
  | "triage"
  | "community_review"
  | "private_security_review"
  | "maintainer_review"
  | "accepted"
  | "rejected"
  | "implemented"
  | "verified"
  | "withdrawn";

export type ContributionKind =
  | "documentation"
  | "bug"
  | "security"
  | "code"
  | "test"
  | "ux"
  | "accessibility"
  | "validator_ops"
  | "publisher_ops"
  | "governance"
  | "research";

export type ContributionTargetType =
  | "repository"
  | "issue"
  | "pull_request"
  | "commit"
  | "document"
  | "package"
  | "transaction"
  | "validator"
  | "network"
  | "incident"
  | "governance_proposal";

export interface ArenaActor {
  id: string;
  role: ArenaRole;
  identityVerified: boolean;
}

export interface ArenaContribution {
  id: string;
  actorId: string;
  kind: ContributionKind;
  visibility: ContributionVisibility;
  status: ContributionStatus;
  title: string;
  description: string;
  targetType: ContributionTargetType;
  targetRef: string;
  sourceRevision?: string;
  createdAt: string;
  updatedAt: string;
}

export interface CreateContributionInput {
  kind: ContributionKind;
  visibility?: ContributionVisibility;
  title: string;
  description: string;
  targetType: ContributionTargetType;
  targetRef: string;
  sourceRevision?: string;
}

export class ArenaDomainError extends Error {
  constructor(
    public readonly code: "UNAUTHENTICATED" | "FORBIDDEN" | "NOT_FOUND" | "INVALID_INPUT" | "INVALID_TRANSITION" | "CONFLICT",
    message: string,
  ) {
    super(message);
    this.name = "ArenaDomainError";
  }
}

const transitions: Readonly<Record<ContributionStatus, readonly ContributionStatus[]>> = {
  draft: ["submitted", "withdrawn"],
  submitted: ["triage", "withdrawn"],
  triage: ["community_review", "private_security_review", "maintainer_review", "rejected"],
  community_review: ["maintainer_review", "rejected", "withdrawn"],
  private_security_review: ["maintainer_review", "rejected", "withdrawn"],
  maintainer_review: ["accepted", "rejected"],
  accepted: ["implemented"],
  rejected: [],
  implemented: ["verified"],
  verified: [],
  withdrawn: [],
};

const staffRoles = new Set<ArenaRole>(["reviewer", "security_reviewer", "maintainer", "moderator", "admin"]);
const securityRoles = new Set<ArenaRole>(["security_reviewer", "maintainer", "admin"]);

export function normalizeCreateContribution(actor: ArenaActor, input: CreateContributionInput): Omit<ArenaContribution, "id" | "createdAt" | "updatedAt"> {
  requireVerifiedActor(actor);
  const title = boundedText(input.title, "title", 8, 180);
  const description = boundedText(input.description, "description", 40, 10_000);
  const targetRef = boundedText(input.targetRef, "targetRef", 1, 1_000);
  const sourceRevision = optionalBoundedText(input.sourceRevision, "sourceRevision", 255);
  const visibility = input.kind === "security" ? "private_security" : input.visibility || "public";

  if (visibility === "private_security" && input.kind !== "security") {
    throw new ArenaDomainError("INVALID_INPUT", "Only security contributions may use private-security visibility.");
  }

  return {
    actorId: actor.id,
    kind: input.kind,
    visibility,
    status: "draft",
    title,
    description,
    targetType: input.targetType,
    targetRef,
    ...(sourceRevision ? { sourceRevision } : {}),
  };
}

export function requireVerifiedActor(actor: ArenaActor) {
  if (actor.role === "guest" || !actor.identityVerified) {
    throw new ArenaDomainError("UNAUTHENTICATED", "A verified contributor identity is required.");
  }
}

export function canViewContribution(actor: ArenaActor | null, contribution: ArenaContribution) {
  if (contribution.visibility === "public") return true;
  if (!actor) return false;
  return actor.id === contribution.actorId || securityRoles.has(actor.role);
}

export function assertCanViewContribution(actor: ArenaActor | null, contribution: ArenaContribution) {
  if (!canViewContribution(actor, contribution)) {
    throw new ArenaDomainError("NOT_FOUND", "Contribution not found.");
  }
}

export function assertCanTransition(actor: ArenaActor, contribution: ArenaContribution, next: ContributionStatus) {
  requireVerifiedActor(actor);
  if (!transitions[contribution.status].includes(next)) {
    throw new ArenaDomainError("INVALID_TRANSITION", `Cannot transition ${contribution.status} to ${next}.`);
  }

  const isOwnerAction = actor.id === contribution.actorId && (
    (contribution.status === "draft" && next === "submitted") ||
    next === "withdrawn"
  );
  if (isOwnerAction) return;

  if (contribution.visibility === "private_security" && !securityRoles.has(actor.role)) {
    throw new ArenaDomainError("NOT_FOUND", "Contribution not found.");
  }

  if (["accepted", "rejected", "implemented", "verified"].includes(next)) {
    if (actor.role !== "maintainer" && actor.role !== "admin") {
      throw new ArenaDomainError("FORBIDDEN", "A maintainer role is required for this decision.");
    }
    return;
  }

  if (!staffRoles.has(actor.role)) {
    throw new ArenaDomainError("FORBIDDEN", "A reviewer role is required for this transition.");
  }
}

export function assertCanVote(actor: ArenaActor, contribution: ArenaContribution) {
  requireVerifiedActor(actor);
  if (contribution.visibility !== "public") {
    throw new ArenaDomainError("NOT_FOUND", "Contribution not found.");
  }
  if (actor.id === contribution.actorId) {
    throw new ArenaDomainError("FORBIDDEN", "Contributors cannot vote on their own submission.");
  }
  if (!["community_review", "maintainer_review"].includes(contribution.status)) {
    throw new ArenaDomainError("INVALID_TRANSITION", "Voting is not open for this contribution.");
  }
}

export function reputationOutcome(status: ContributionStatus): { eventType: "accepted" | "implemented" | "verified"; points: number } | null {
  switch (status) {
    case "accepted": return { eventType: "accepted", points: 25 };
    case "implemented": return { eventType: "implemented", points: 50 };
    case "verified": return { eventType: "verified", points: 75 };
    default: return null;
  }
}

function boundedText(value: unknown, field: string, minimum: number, maximum: number) {
  if (typeof value !== "string") throw new ArenaDomainError("INVALID_INPUT", `${field} must be text.`);
  const normalized = value.trim();
  if (normalized.length < minimum || normalized.length > maximum) {
    throw new ArenaDomainError("INVALID_INPUT", `${field} must contain between ${minimum} and ${maximum} characters.`);
  }
  return normalized;
}

function optionalBoundedText(value: unknown, field: string, maximum: number) {
  if (value === undefined || value === null || value === "") return undefined;
  return boundedText(value, field, 1, maximum);
}
