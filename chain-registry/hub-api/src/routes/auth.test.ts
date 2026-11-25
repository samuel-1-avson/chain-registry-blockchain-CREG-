import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { Hono } from "hono";
import { HubDatabase } from "../db/index.js";
import {
  createSession,
  loadSession,
  purgeExpiredSessions,
  setSessionCookie,
  type SessionVariables,
} from "../middleware/session.js";
import { createAuthRoutes } from "./auth.js";

function withTempDb(fn: (db: HubDatabase) => Promise<void> | void) {
  return async () => {
    const dir = mkdtempSync(join(tmpdir(), "creg-hub-auth-"));
    const db = new HubDatabase(join(dir, "hub.db"));
    try {
      db.connect();
      await fn(db);
    } finally {
      db.close();
      rmSync(dir, { recursive: true, force: true });
    }
  };
}

test(
  "ECA-40: SIWE verify consumes nonce atomically before signature verify and returns generic errors",
  withTempDb(async (db) => {
    const app = new Hono<{ Variables: SessionVariables }>();
    app.route(
      "/api/auth",
      createAuthRoutes({
        db,
        siweDomain: "testnet.cregnet.dev",
        siweUri: "https://testnet.cregnet.dev",
        nonceTtlSec: 300,
        sessionMaxAgeSec: 3600,
        secureCookies: true,
      }),
    );

    const address = "0x000000000000000000000000000000000000dEaD";
    const nonceRes = await app.request(`/api/auth/nonce?address=${address}`);
    assert.equal(nonceRes.status, 200);
    const { nonce, message } = (await nonceRes.json()) as {
      nonce: string;
      message: string;
    };

    const bogusSig = `0x${"11".repeat(65)}`;
    const first = await app.request("/api/auth/verify", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ message, signature: bogusSig }),
    });
    assert.equal(first.status, 401);
    const firstBody = (await first.json()) as { error: string; message: string };
    assert.equal(firstBody.error, "authentication_failed");
    assert.equal(firstBody.message, "Authentication failed.");

    // Nonce must already be marked consumed even though signature verification failed.
    const row = db.handle
      .prepare("SELECT consumed_at FROM nonces WHERE nonce = ?")
      .get(nonce) as { consumed_at: string | null } | undefined;
    assert.ok(row?.consumed_at, "nonce should be consumed before signature check");

    const replay = await app.request("/api/auth/verify", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ message, signature: bogusSig }),
    });
    assert.equal(replay.status, 401);
    const replayBody = (await replay.json()) as { error: string; message: string };
    assert.equal(replayBody.error, "authentication_failed");
    assert.equal(replayBody.message, "Authentication failed.");
  }),
);

test(
  "ECA-40: purgeExpiredSessions deletes expired rows and session cookies enforce Secure + HttpOnly",
  withTempDb(async (db) => {
    const active = createSession(db, "0x0000000000000000000000000000000000000001", 3600);
    const expired = createSession(db, "0x0000000000000000000000000000000000000002", -60);

    const removed = purgeExpiredSessions(db);
    assert.equal(removed, 1);
    assert.ok(loadSession(db, active.id));
    assert.equal(loadSession(db, expired.id), null);

    const cookieApp = new Hono();
    cookieApp.get("/set", (c) => {
      setSessionCookie(c, active.id, 3600, true);
      return c.text("ok");
    });
    const res = await cookieApp.request("/set");
    const setCookie = res.headers.get("set-cookie") ?? "";
    assert.match(setCookie, /HttpOnly/i);
    assert.match(setCookie, /Secure/i);
    assert.match(setCookie, /SameSite=Lax/i);
  }),
);
