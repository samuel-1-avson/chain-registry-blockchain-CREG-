import { Hono } from "hono";
import { randomUUID } from "node:crypto";
import type { HubDatabase } from "../db/index.js";
import type { SessionVariables } from "../middleware/session.js";
import {
  ArenaDomainError,
  type ContributionStatus,
  type CreateContributionInput,
} from "../contributions/domain.js";
import type { ContributionService } from "../contributions/service.js";
import type { SqliteContributionRepository } from "../contributions/sqliteRepository.js";

type ContributionRouteOptions = {
  db: HubDatabase;
  service: ContributionService;
  repository: SqliteContributionRepository;
};

function mapDomainError(error: ArenaDomainError): { status: 400 | 401 | 403 | 404 | 409; body: Record<string, string> } {
  switch (error.code) {
    case "UNAUTHENTICATED":
      return { status: 401, body: { error: error.code, message: error.message } };
    case "FORBIDDEN":
      return { status: 403, body: { error: error.code, message: error.message } };
    case "NOT_FOUND":
      return { status: 404, body: { error: error.code, message: error.message } };
    case "CONFLICT":
      return { status: 409, body: { error: error.code, message: error.message } };
    default:
      return { status: 400, body: { error: error.code, message: error.message } };
  }
}

function requestId(header: string | undefined): string {
  return header?.trim() || randomUUID();
}

export function createContributionRoutes(options: ContributionRouteOptions) {
  const { db, service, repository } = options;
  const app = new Hono<{ Variables: SessionVariables }>();

  app.use("*", async (c, next) => {
    if (db.getState() !== "ready") {
      return c.json(
        { error: "service_unavailable", message: "Database not ready." },
        503,
      );
    }
    await next();
  });

  app.post("/", async (c) => {
    try {
      const session = c.get("session");
      if (!session) {
        throw new ArenaDomainError("UNAUTHENTICATED", "Authentication is required.");
      }
      const actor = repository.ensureActorForEthereumAddress(session.address);
      const body = (await c.req.json()) as CreateContributionInput;
      const contribution = await service.create(
        { actor, requestId: requestId(c.req.header("x-request-id")) },
        body,
      );
      return c.json({ contribution }, 201);
    } catch (error) {
      if (error instanceof ArenaDomainError) {
        const mapped = mapDomainError(error);
        return c.json(mapped.body, mapped.status);
      }
      throw error;
    }
  });

  app.get("/:id", async (c) => {
    try {
      const session = c.get("session");
      const actor = session
        ? repository.ensureActorForEthereumAddress(session.address)
        : null;
      const contribution = await service.get(
        { actor, requestId: requestId(c.req.header("x-request-id")) },
        c.req.param("id"),
      );
      return c.json({ contribution });
    } catch (error) {
      if (error instanceof ArenaDomainError) {
        const mapped = mapDomainError(error);
        return c.json(mapped.body, mapped.status);
      }
      throw error;
    }
  });

  app.post("/:id/transition", async (c) => {
    try {
      const session = c.get("session");
      if (!session) {
        throw new ArenaDomainError("UNAUTHENTICATED", "Authentication is required.");
      }
      const actor = repository.ensureActorForEthereumAddress(session.address);
      const body = (await c.req.json()) as { status?: ContributionStatus };
      if (!body.status) {
        throw new ArenaDomainError("INVALID_INPUT", "status is required.");
      }
      const contribution = await service.transition(
        { actor, requestId: requestId(c.req.header("x-request-id")) },
        c.req.param("id"),
        body.status,
      );
      return c.json({ contribution });
    } catch (error) {
      if (error instanceof ArenaDomainError) {
        const mapped = mapDomainError(error);
        return c.json(mapped.body, mapped.status);
      }
      throw error;
    }
  });

  app.post("/:id/vote", async (c) => {
    try {
      const session = c.get("session");
      if (!session) {
        throw new ArenaDomainError("UNAUTHENTICATED", "Authentication is required.");
      }
      const actor = repository.ensureActorForEthereumAddress(session.address);
      const body = (await c.req.json()) as { vote?: -1 | 1 };
      const result = await service.vote(
        { actor, requestId: requestId(c.req.header("x-request-id")) },
        c.req.param("id"),
        body.vote as -1 | 1,
      );
      return c.json({ result });
    } catch (error) {
      if (error instanceof ArenaDomainError) {
        const mapped = mapDomainError(error);
        return c.json(mapped.body, mapped.status);
      }
      throw error;
    }
  });

  return app;
}
