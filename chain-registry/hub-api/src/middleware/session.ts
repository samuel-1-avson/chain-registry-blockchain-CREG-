import type { Context, MiddlewareHandler } from "hono";
import { getCookie, setCookie, deleteCookie } from "hono/cookie";
import { randomBytes } from "node:crypto";
import type { HubDatabase } from "../db/index.js";
import { normalizeAddress } from "../db/index.js";

export const SESSION_COOKIE = "hub_session";

export type SessionRecord = {
  id: string;
  address: string;
  createdAt: string;
  expiresAt: string;
};

export type SessionVariables = {
  session: SessionRecord | null;
};

type SessionMiddlewareOptions = {
  db: HubDatabase;
  maxAgeSec: number;
  secure: boolean;
};

function isoAfterSeconds(seconds: number): string {
  return new Date(Date.now() + seconds * 1000).toISOString();
}

export function createSessionId(): string {
  return randomBytes(32).toString("hex");
}

export function createSession(
  db: HubDatabase,
  address: string,
  maxAgeSec: number,
): SessionRecord {
  const normalized = normalizeAddress(address);
  const id = createSessionId();
  const createdAt = new Date().toISOString();
  const expiresAt = isoAfterSeconds(maxAgeSec);

  db.handle
    .prepare(
      `INSERT INTO sessions (id, address, created_at, expires_at)
       VALUES (?, ?, ?, ?)`,
    )
    .run(id, normalized, createdAt, expiresAt);

  return { id, address: normalized, createdAt, expiresAt };
}

export function deleteSession(db: HubDatabase, sessionId: string): void {
  db.handle.prepare("DELETE FROM sessions WHERE id = ?").run(sessionId);
}

export function loadSession(
  db: HubDatabase,
  sessionId: string,
): SessionRecord | null {
  const row = db.handle
    .prepare(
      `SELECT id, address, created_at, expires_at
       FROM sessions
       WHERE id = ?`,
    )
    .get(sessionId) as
    | {
        id: string;
        address: string;
        created_at: string;
        expires_at: string;
      }
    | undefined;

  if (!row) return null;

  if (Date.parse(row.expires_at) <= Date.now()) {
    deleteSession(db, row.id);
    return null;
  }

  return {
    id: row.id,
    address: row.address,
    createdAt: row.created_at,
    expiresAt: row.expires_at,
  };
}

export function setSessionCookie(
  c: Context,
  sessionId: string,
  maxAgeSec: number,
  secure: boolean,
): void {
  setCookie(c, SESSION_COOKIE, sessionId, {
    httpOnly: true,
    secure,
    sameSite: "Lax",
    path: "/",
    maxAge: maxAgeSec,
  });
}

export function clearSessionCookie(c: Context, secure: boolean): void {
  deleteCookie(c, SESSION_COOKIE, {
    httpOnly: true,
    secure,
    sameSite: "Lax",
    path: "/",
  });
}

export function sessionMiddleware(
  options: SessionMiddlewareOptions,
): MiddlewareHandler<{ Variables: SessionVariables }> {
  const { db, secure } = options;

  return async (c, next) => {
    const sessionId = getCookie(c, SESSION_COOKIE);
    if (sessionId && db.getState() === "ready") {
      c.set("session", loadSession(db, sessionId));
    } else {
      c.set("session", null);
    }
    await next();
  };
}

export function requireSession(): MiddlewareHandler<{
  Variables: SessionVariables;
}> {
  return async (c, next) => {
    const session = c.get("session");
    if (!session) {
      return c.json({ error: "unauthorized", message: "Sign in required." }, 401);
    }
    await next();
  };
}
