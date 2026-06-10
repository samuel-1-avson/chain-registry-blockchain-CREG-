import { serve } from "@hono/node-server";
import { Hono } from "hono";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";

const port = Number(process.env.HUB_API_PORT ?? process.env.PORT ?? "8095");
const dbPath = process.env.HUB_DB_PATH ?? "/data/hub.db";

// Ensure SQLite parent directory exists (Phase 0: volume mount only; schema in Phase 2).
try {
  mkdirSync(dirname(dbPath), { recursive: true });
} catch {
  // Non-fatal for health-only scaffold.
}

const app = new Hono();

app.get("/api/health", (c) =>
  c.json({
    status: "ok",
    service: "hub-api",
    phase: "1",
    dbPathConfigured: Boolean(dbPath),
  }),
);

serve({ fetch: app.fetch, port }, (info) => {
  console.log(`hub-api listening on http://127.0.0.1:${info.port}`);
});
