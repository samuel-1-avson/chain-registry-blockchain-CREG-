import { Hono } from "hono";
import { randomBytes } from "node:crypto";
import { SiweMessage } from "siwe";
import type { HubDatabase } from "../db/index.js";
import { normalizeAddress } from "../db/index.js";
import {
  clearSessionCookie,
  createSession,
  deleteSession,
  loadSession,
  setSessionCookie,
  type SessionVariables,
} from "../middleware/session.js";

type AuthRouteOptions = {
  db: HubDatabase;
  siweDomain: string;
  siweUri: string;
  nonceTtlSec: number;
  sessionMaxAgeSec: number;
  secureCookies: boolean;
};

function isoAfterSeconds(seconds: number): string {
  return new Date(Date.now() + seconds * 1000).toISOString();
}

function createNonceValue(): string {
  return randomBytes(16).toString("hex");
}

function purgeExpiredNonces(db: HubDatabase): void {
  db.handle
    .prepare("DELETE FROM nonces WHERE expires_at <= ?")
    .run(new Date().toISOString());
}

export function createAuthRoutes(options: AuthRouteOptions) {
  const {
    db,
    siweDomain,
    siweUri,
    nonceTtlSec,
    sessionMaxAgeSec,
    secureCookies,
  } = options;

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

  app.get("/nonce", (c) => {
    const address = c.req.query("address");
    if (!address || !/^0x[a-fA-F0-9]{40}$/.test(address.trim())) {
      return c.json(
        { error: "invalid_address", message: "Provide a valid 0x address." },
        400,
      );
    }

    purgeExpiredNonces(db);
    const normalized = normalizeAddress(address);
    const nonce = createNonceValue();
    const issuedAt = new Date().toISOString();
    const expiresAt = isoAfterSeconds(nonceTtlSec);

    db.handle
      .prepare(
        `INSERT INTO nonces (nonce, address, issued_at, expires_at, consumed_at)
         VALUES (?, ?, ?, ?, NULL)`,
      )
      .run(nonce, normalized, issuedAt, expiresAt);

    const message = new SiweMessage({
      domain: siweDomain,
      address: address.trim(),
      statement: "Sign in to the CREG testnet hub.",
      uri: siweUri,
      version: "1",
      chainId: 11155111,
      nonce,
    }).prepareMessage();

    return c.json({
      nonce,
      message,
      expiresAt,
      domain: siweDomain,
      chainId: 11155111,
    });
  });

  app.post("/verify", async (c) => {
    let body: { message?: string; signature?: string };
    try {
      body = await c.req.json();
    } catch {
      return c.json({ error: "invalid_json", message: "Invalid JSON body." }, 400);
    }

    const message = body.message?.trim();
    const signature = body.signature?.trim();
    if (!message || !signature) {
      return c.json(
        {
          error: "invalid_request",
          message: "Both message and signature are required.",
        },
        400,
      );
    }

    let siweMessage: SiweMessage;
    try {
      siweMessage = new SiweMessage(message);
    } catch {
      return c.json({ error: "invalid_message", message: "Invalid SIWE message." }, 400);
    }

    if (siweMessage.domain !== siweDomain) {
      return c.json({ error: "invalid_domain", message: "SIWE domain mismatch." }, 401);
    }

    if (siweMessage.chainId !== 11155111) {
      return c.json({ error: "invalid_chain", message: "Switch to Sepolia." }, 401);
    }

    const nonceRow = db.handle
      .prepare(
        `SELECT nonce, address, expires_at, consumed_at
         FROM nonces
         WHERE nonce = ?`,
      )
      .get(siweMessage.nonce) as
      | {
          nonce: string;
          address: string;
          expires_at: string;
          consumed_at: string | null;
        }
      | undefined;

    if (!nonceRow || nonceRow.consumed_at) {
      return c.json({ error: "invalid_nonce", message: "Nonce expired or already used." }, 401);
    }

    if (Date.parse(nonceRow.expires_at) <= Date.now()) {
      return c.json({ error: "invalid_nonce", message: "Nonce expired or already used." }, 401);
    }

    if (normalizeAddress(siweMessage.address) !== nonceRow.address) {
      return c.json({ error: "invalid_nonce", message: "Nonce address mismatch." }, 401);
    }

    try {
      await siweMessage.verify({ signature, domain: siweDomain, nonce: siweMessage.nonce });
    } catch (error) {
      const detail = error instanceof Error ? error.message : "Signature verification failed.";
      return c.json({ error: "invalid_signature", message: detail }, 401);
    }

    db.handle
      .prepare("UPDATE nonces SET consumed_at = ? WHERE nonce = ?")
      .run(new Date().toISOString(), siweMessage.nonce);

    const session = createSession(db, siweMessage.address, sessionMaxAgeSec);
    setSessionCookie(c, session.id, sessionMaxAgeSec, secureCookies);

    return c.json({
      ok: true,
      address: session.address,
      expiresAt: session.expiresAt,
    });
  });

  app.post("/logout", (c) => {
    const sessionId = c.get("session")?.id;
    if (sessionId) {
      deleteSession(db, sessionId);
    }
    clearSessionCookie(c, secureCookies);
    return c.json({ ok: true });
  });

  app.get("/session", (c) => {
    const sessionId = c.req.header("cookie")?.match(/hub_session=([^;]+)/)?.[1];
    const session = sessionId ? loadSession(db, sessionId) : c.get("session");
    if (!session) {
      return c.json({ authenticated: false });
    }
    return c.json({
      authenticated: true,
      address: session.address,
      expiresAt: session.expiresAt,
    });
  });

  return app;
}
