import {
  ArenaDomainError,
  assertCanTransition,
  assertCanViewContribution,
  assertCanVote,
  normalizeCreateContribution,
  reputationOutcome,
  type ArenaActor,
  type ContributionStatus,
  type CreateContributionInput,
} from "./domain";
import type { ContributionRepository } from "./repository";

export interface ArenaRequestContext {
  actor: ArenaActor | null;
  requestId?: string;
}

export class ContributionService {
  constructor(
    private readonly repository: ContributionRepository,
    private readonly reputationPolicyVersion = "arena-reputation-v1",
  ) {}

  async create(context: ArenaRequestContext, input: CreateContributionInput) {
    if (!context.actor) throw new ArenaDomainError("UNAUTHENTICATED", "Authentication is required.");
    const normalized = normalizeCreateContribution(context.actor, input);
    return this.repository.withTransaction(async repository => {
      const contribution = await repository.createContribution({
        ...input,
        ...normalized,
      });
      await repository.appendAudit({
        actorId: context.actor!.id,
        contributionId: contribution.id,
        action: "contribution.created",
        nextState: contribution,
        requestId: context.requestId,
      });
      return contribution;
    });
  }

  async get(context: ArenaRequestContext, id: string) {
    const contribution = await this.repository.findContributionById(id, { includePrivate: Boolean(context.actor) });
    if (!contribution) throw new ArenaDomainError("NOT_FOUND", "Contribution not found.");
    assertCanViewContribution(context.actor, contribution);
    return contribution;
  }

  async transition(context: ArenaRequestContext, id: string, nextStatus: ContributionStatus) {
    if (!context.actor) throw new ArenaDomainError("UNAUTHENTICATED", "Authentication is required.");
    return this.repository.withTransaction(async repository => {
      const current = await repository.findContributionById(id, { includePrivate: true });
      if (!current) throw new ArenaDomainError("NOT_FOUND", "Contribution not found.");
      assertCanViewContribution(context.actor, current);
      assertCanTransition(context.actor, current, nextStatus);

      const updated = await repository.updateContributionStatus(id, current.status, nextStatus);
      if (!updated) throw new ArenaDomainError("CONFLICT", "The contribution changed while it was being reviewed.");

      await repository.appendAudit({
        actorId: context.actor.id,
        contributionId: id,
        action: `contribution.${nextStatus}`,
        previousState: current,
        nextState: updated,
        requestId: context.requestId,
      });

      const outcome = reputationOutcome(nextStatus);
      if (outcome) {
        await repository.appendReputation({
          actorId: current.actorId,
          contributionId: current.id,
          eventType: outcome.eventType,
          points: outcome.points,
          policyVersion: this.reputationPolicyVersion,
          reason: `Contribution reached ${nextStatus} under ${this.reputationPolicyVersion}.`,
          awardedBy: context.actor.id,
        });
      }
      return updated;
    });
  }

  async vote(context: ArenaRequestContext, id: string, vote: -1 | 1) {
    if (!context.actor) throw new ArenaDomainError("UNAUTHENTICATED", "Authentication is required.");
    if (vote !== -1 && vote !== 1) throw new ArenaDomainError("INVALID_INPUT", "Vote must be -1 or 1.");
    const contribution = await this.get(context, id);
    assertCanVote(context.actor, contribution);
    const result = await this.repository.upsertVote({ contributionId: id, actorId: context.actor.id, vote });
    await this.repository.appendAudit({
      actorId: context.actor.id,
      contributionId: id,
      action: result === "created" ? "vote.created" : "vote.updated",
      requestId: context.requestId,
    });
    return result;
  }
}
