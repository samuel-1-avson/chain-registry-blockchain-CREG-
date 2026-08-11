import assert from "node:assert/strict";
import test from "node:test";
import {
  assertCanTransition,
  assertCanVote,
  normalizeCreateContribution,
  reputationOutcome,
  type ArenaActor,
  type ArenaContribution,
} from "./domain.js";

const contributor: ArenaActor = {
  id: "actor-1",
  role: "contributor",
  identityVerified: true,
};

const maintainer: ArenaActor = {
  id: "maintainer-1",
  role: "maintainer",
  identityVerified: true,
};

const baseContribution: ArenaContribution = {
  id: "c1",
  actorId: contributor.id,
  kind: "documentation",
  visibility: "public",
  status: "draft",
  title: "Clarify sandbox admission",
  description: "The whitepaper section on sandbox admission should cite the current gate list.",
  targetType: "document",
  targetRef: "whitepaper#sandbox",
  createdAt: "2026-08-11T00:00:00.000Z",
  updatedAt: "2026-08-11T00:00:00.000Z",
};

test("normalizeCreateContribution forces security visibility for security kind", () => {
  const normalized = normalizeCreateContribution(contributor, {
    kind: "security",
    title: "Private disclosure title",
    description: "Detailed private security disclosure with enough characters to pass validation.",
    targetType: "repository",
    targetRef: "chain-registry",
    visibility: "public",
  });
  assert.equal(normalized.visibility, "private_security");
  assert.equal(normalized.status, "draft");
});

test("owner can submit draft and maintainer can accept from maintainer_review", () => {
  assert.doesNotThrow(() => assertCanTransition(contributor, baseContribution, "submitted"));
  const inReview = { ...baseContribution, status: "maintainer_review" as const };
  assert.doesNotThrow(() => assertCanTransition(maintainer, inReview, "accepted"));
  assert.equal(reputationOutcome("accepted")?.points, 25);
});

test("contributors cannot vote on their own contribution", () => {
  const open = { ...baseContribution, status: "community_review" as const };
  assert.throws(() => assertCanVote(contributor, open));
  assert.doesNotThrow(() =>
    assertCanVote({ id: "voter-2", role: "contributor", identityVerified: true }, open),
  );
});
