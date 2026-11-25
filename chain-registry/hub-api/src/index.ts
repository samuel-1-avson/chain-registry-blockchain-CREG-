import { serve } from "@hono/node-server";
import { Hono } from "hono";
import { HubDatabase } from "./db/index.js";
import { rateLimit } from "./middleware/rateLimit.js";
import { singleFlightCache } from "./status/singleFlight.js";
import {
  purgeExpiredSessions,
  sessionMiddleware,
  type SessionVariables,
} from "./middleware/session.js";
import { createAuthRoutes } from "./routes/auth.js";
import { createContributionRoutes } from "./routes/contributions.js";
import { ContributionService } from "./contributions/service.js";
import { SqliteContributionRepository } from "./contributions/sqliteRepository.js";

const IMPLEMENTATION_PHASE = "1";

const port = Number(process.env.HUB_API_PORT ?? process.env.PORT ?? "8095");
const dbPath = process.env.HUB_DB_PATH ?? "/data/hub.db";
const publicNodeApiUrl =
  process.env.HUB_NODE_API_URL ??
  process.env.CREG_NODE_API_URL ??
  "https://api.testnet.cregnet.dev";
const publicFaucetUrl =
  process.env.HUB_FAUCET_URL ?? "https://faucet.testnet.cregnet.dev";
const publicExplorerUrl =
  process.env.HUB_EXPLORER_URL ?? "https://explorer.testnet.cregnet.dev";
const publicSpecUrl =
  process.env.HUB_SPEC_URL ??
  "https://spec.testnet.cregnet.dev/chain-spec.json";
const probeTimeoutMs = Number(process.env.HUB_STATUS_TIMEOUT_MS ?? "3500");
const sessionMaxAgeSec = Number(process.env.HUB_SESSION_MAX_AGE_SEC ?? "86400");
const nonceTtlSec = Number(process.env.HUB_NONCE_TTL_SEC ?? "300");
const siweDomain =
  process.env.HUB_SIWE_DOMAIN ?? process.env.CREG_PUBLIC_JOIN_HOST ?? "testnet.cregnet.dev";
const siweUri =
  process.env.HUB_SIWE_URI ?? `https://${siweDomain.replace(/^https?:\/\//, "")}`;
const secureCookies = process.env.HUB_COOKIE_SECURE !== "false";
const indexerApiUrl = (process.env.HUB_INDEXER_URL ?? "").trim();
const statusRateLimitWindowMs = Number(
  process.env.HUB_STATUS_RATE_WINDOW_MS ?? "60000",
);
const statusRateLimitMax = Number(process.env.HUB_STATUS_RATE_MAX ?? "30");
const authRateLimitWindowMs = Number(process.env.HUB_AUTH_RATE_WINDOW_MS ?? "60000");
const authRateLimitMax = Number(process.env.HUB_AUTH_RATE_MAX ?? "20");
const contributionRateLimitWindowMs = Number(
  process.env.HUB_CONTRIBUTION_RATE_WINDOW_MS ?? "60000",
);
const contributionRateLimitMax = Number(process.env.HUB_CONTRIBUTION_RATE_MAX ?? "40");
// ECA-36: one upstream fan-out per window, shared by concurrent callers.
const statusCacheTtlMs = Math.min(
  Math.max(Number(process.env.HUB_STATUS_CACHE_MS ?? "7500") || 7500, 1000),
  10000,
);

type UpstreamProbe = {
  name: string;
  url: string;
  ok: boolean;
  statusCode?: number;
  latencyMs: number;
  message?: string;
};

type PublicProbe = {
  name: string;
  ok: boolean;
  statusCode?: number;
  latencyMs: number;
  message?: string;
};

type ProbeWithData = {
  probe: UpstreamProbe;
  data: unknown;
};

const hubDb = new HubDatabase(dbPath);
let dbInitError: string | undefined;

try {
  hubDb.connect();
} catch (error) {
  dbInitError = error instanceof Error ? error.message : "Database connect failed";
  console.error(`hub-api database init failed: ${dbInitError}`);
}

const app = new Hono<{ Variables: SessionVariables }>();

app.use("*", sessionMiddleware({ db: hubDb, maxAgeSec: sessionMaxAgeSec, secure: secureCookies }));

app.get("/api/health", (c) => {
  const dbState = hubDb.getState();
  const body: Record<string, unknown> = {
    status: dbState === "error" ? "degraded" : "ok",
    service: "hub-api",
    phase: IMPLEMENTATION_PHASE,
    db: dbState,
    arenaAuthority: "firebase",
  };

  if (dbState === "ready") {
    try {
      const migrationCount = hubDb.handle
        .prepare("SELECT COUNT(*) AS count FROM schema_migrations")
        .get() as { count: number };
      body.migrationsApplied = migrationCount.count;
    } catch {
      body.db = "error";
      body.status = "degraded";
    }
  }

  if (dbInitError) {
    body.dbError = dbInitError;
  }

  return c.json(body, dbState === "error" ? 503 : 200);
});

const authRoutes = createAuthRoutes({
  db: hubDb,
  siweDomain: siweDomain.replace(/^https?:\/\//, ""),
  siweUri,
  nonceTtlSec,
  sessionMaxAgeSec,
  secureCookies,
});

authRoutes.use(
  "*",
  rateLimit({
    windowMs: authRateLimitWindowMs,
    max: authRateLimitMax,
    keyPrefix: "auth",
  }),
);

app.route("/api/auth", authRoutes);

if (hubDb.getState() === "ready") {
  const contributionRepository = new SqliteContributionRepository(hubDb.handle);
  const contributionService = new ContributionService(contributionRepository);
  const contributionRoutes = createContributionRoutes({
    db: hubDb,
    service: contributionService,
    repository: contributionRepository,
  });
  contributionRoutes.use(
    "*",
    rateLimit({
      windowMs: contributionRateLimitWindowMs,
      max: contributionRateLimitMax,
      keyPrefix: "contributions",
    }),
  );
  app.route("/api/v1/contributions", contributionRoutes);
}

const publicStatus = singleFlightCache(statusCacheTtlMs, buildPublicStatus);
const publicIndexSummary = singleFlightCache(statusCacheTtlMs, buildIndexSummary);

const SESSION_PURGE_INTERVAL_MS = 15 * 60 * 1000;
if (hubDb.getState() === "ready") {
  setInterval(() => {
    try {
      purgeExpiredSessions(hubDb);
      hubDb.handle
        .prepare("DELETE FROM nonces WHERE expires_at <= ?")
        .run(new Date().toISOString());
    } catch (error) {
      console.error("hub-api periodic purge failed:", error);
    }
  }, SESSION_PURGE_INTERVAL_MS).unref();
}

const indexRateLimit = rateLimit({
  windowMs: statusRateLimitWindowMs,
  max: statusRateLimitMax,
  keyPrefix: "index-read",
});

app.get(
  "/api/v1/index/summary",
  indexRateLimit,
  async (c) => {
    c.header("Cache-Control", `public, max-age=${Math.floor(statusCacheTtlMs / 1000)}`);
    return c.json(await publicIndexSummary.get());
  },
);

app.get("/api/v1/index/quarantine-queue", indexRateLimit, async (c) => {
  const limit = parseBoundedInt(c.req.query("limit"), 25, 1, 200);
  return proxyIndexerJson(c, `/v1/index/quarantine-queue?limit=${limit}`);
});

app.get("/api/v1/index/bounties/leaderboard", indexRateLimit, async (c) => {
  const limit = parseBoundedInt(c.req.query("limit"), 25, 1, 200);
  return proxyIndexerJson(c, `/v1/index/bounties/leaderboard?limit=${limit}`);
});

app.get("/api/v1/index/committee-diversity", indexRateLimit, async (c) => {
  const epoch = parseBoundedInt(c.req.query("epoch"), 0, 0, Number.MAX_SAFE_INTEGER);
  return proxyIndexerJson(c, `/v1/index/committee-diversity?epoch=${epoch}`);
});

app.get(
  "/api/status/public",
  rateLimit({
    windowMs: statusRateLimitWindowMs,
    max: statusRateLimitMax,
    keyPrefix: "status-public",
  }),
  async (c) => {
    c.header("Cache-Control", `public, max-age=${Math.floor(statusCacheTtlMs / 1000)}`);
    return c.json(await publicStatus.get());
  },
);

// Return type is inferred so `c.json` sees the same literal JSON shape the
// handler returned before it was cached.
async function buildPublicStatus() {
    const checkedAt = new Date().toISOString();
    const [nodeHealth, nodeStats, faucetHealth, faucetStats, spec, explorer] =
      await Promise.all([
        fetchJsonProbe("node_api", joinUrl(publicNodeApiUrl, "/v1/public/health")),
        fetchJsonProbe(
          "node_chain_stats",
          joinUrl(publicNodeApiUrl, "/v1/public/chain/stats"),
        ),
        fetchJsonProbe("faucet", joinUrl(publicFaucetUrl, "/health")),
        fetchJsonProbe("faucet_stats", joinUrl(publicFaucetUrl, "/api/stats")),
        fetchJsonProbe("chain_spec", publicSpecUrl),
        fetchProbe("explorer", publicExplorerUrl),
      ]);

    const probes = [
      nodeHealth.probe,
      nodeStats.probe,
      faucetHealth.probe,
      faucetStats.probe,
      spec.probe,
      explorer,
    ];
    const criticalOk = nodeHealth.probe.ok && faucetHealth.probe.ok && spec.probe.ok;
    const allOk = probes.every((probe) => probe.ok);
    const chainStats = asRecord(nodeStats.data);
    const faucetStatsData = asRecord(faucetStats.data);

    return {
      status: criticalOk && allOk ? "ok" : "degraded",
      service: "hub-api",
      phase: IMPLEMENTATION_PHASE,
      checkedAt,
      upstreams: probes.map(toPublicProbe),
      chain: {
        height:
          pickNumber(chainStats, "current_height") ??
          pickNumber(chainStats, "tip_height"),
        finalizedHeight: pickNumber(chainStats, "finalized_height"),
        validators:
          pickNumber(chainStats, "active_validators") ??
          pickNumber(chainStats, "validator_count"),
        packages: pickNumber(chainStats, "package_count"),
        finalizationLag: pickNumber(chainStats, "finalization_lag"),
      },
      faucet: {
        tokenDripsAvailable: pickBoolean(
          asRecord(faucetHealth.data),
          "token_drips_available",
        ),
        nativeDripsAvailable: pickBoolean(
          asRecord(faucetHealth.data),
          "native_drips_available",
        ),
        totalDrips: pickNestedNumber(faucetStatsData, ["stats", "total_drips"]),
        cooldownSeconds: pickNumber(faucetStatsData, "cooldown_seconds"),
        tokenReserve: pickString(faucetStatsData, "faucet_balance_formatted"),
        nativeReserve: pickString(
          faucetStatsData,
          "faucet_native_balance_formatted",
        ),
      },
    };
}

serve({ fetch: app.fetch, port }, (info) => {
  console.log(`hub-api listening on http://127.0.0.1:${info.port} (phase ${IMPLEMENTATION_PHASE})`);
});

function toPublicProbe(probe: UpstreamProbe): PublicProbe {
  return {
    name: probe.name,
    ok: probe.ok,
    statusCode: probe.statusCode,
    latencyMs: probe.latencyMs,
    message: probe.message,
  };
}

function joinUrl(base: string, path: string): string {
  const cleanBase = base.replace(/\/$/, "");
  const cleanPath = path.startsWith("/") ? path : `/${path}`;
  return `${cleanBase}${cleanPath}`;
}

async function fetchJsonProbe(
  name: string,
  url: string,
): Promise<ProbeWithData> {
  const startedAt = Date.now();
  try {
    const response = await fetchWithTimeout(url);
    const probe = {
      name,
      url,
      ok: response.ok,
      statusCode: response.status,
      latencyMs: Date.now() - startedAt,
      message: response.ok ? undefined : `HTTP ${response.status}`,
    };
    if (!response.ok) {
      return { probe, data: null };
    }
    return {
      probe,
      data: await response.json(),
    };
  } catch (error) {
    return {
      probe: {
        name,
        url,
        ok: false,
        latencyMs: Date.now() - startedAt,
        message: error instanceof Error ? error.message : "Invalid JSON",
      },
      data: null,
    };
  }
}

async function fetchProbe(name: string, url: string): Promise<UpstreamProbe> {
  const startedAt = Date.now();
  try {
    const response = await fetchWithTimeout(url);
    return {
      name,
      url,
      ok: response.ok,
      statusCode: response.status,
      latencyMs: Date.now() - startedAt,
      message: response.ok ? undefined : `HTTP ${response.status}`,
    };
  } catch (error) {
    return {
      name,
      url,
      ok: false,
      latencyMs: Date.now() - startedAt,
      message: error instanceof Error ? error.message : "Upstream unreachable",
    };
  }
}

async function fetchWithTimeout(url: string): Promise<Response> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), probeTimeoutMs);
  try {
    return await fetch(url, {
      headers: { accept: "application/json" },
      signal: controller.signal,
    });
  } finally {
    clearTimeout(timeout);
  }
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function pickNumber(
  source: Record<string, unknown>,
  key: string,
): number | null {
  const value = source[key];
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function pickNestedNumber(
  source: Record<string, unknown>,
  path: string[],
): number | null {
  let current: unknown = source;
  for (const key of path) {
    current = asRecord(current)[key];
  }
  return pickNumber({ value: current }, "value");
}

function pickBoolean(
  source: Record<string, unknown>,
  key: string,
): boolean | null {
  const value = source[key];
  return typeof value === "boolean" ? value : null;
}

function pickString(
  source: Record<string, unknown>,
  key: string,
): string | null {
  const value = source[key];
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  return null;
}

function parseBoundedInt(
  raw: string | undefined,
  fallback: number,
  min: number,
  max: number,
): number {
  if (!raw || !/^\d+$/.test(raw.trim())) return fallback;
  const n = Number(raw.trim());
  if (!Number.isSafeInteger(n)) return fallback;
  return Math.min(Math.max(n, min), max);
}

async function proxyIndexerJson(
  c: import("hono").Context,
  pathAndQuery: string,
) {
  if (!indexerApiUrl) {
    return c.json(
      { error: "indexer_unavailable", message: "Indexer service unavailable." },
      503,
    );
  }
  const result = await fetchJsonProbe("indexer", joinUrl(indexerApiUrl, pathAndQuery));
  if (!result.probe.ok || !result.data) {
    return c.json(
      { error: "indexer_unavailable", message: "Indexer service unavailable." },
      503,
    );
  }
  c.header("Cache-Control", `public, max-age=${Math.floor(statusCacheTtlMs / 1000)}`);
  return c.json(result.data);
}

async function buildIndexSummary() {
  const checkedAt = new Date().toISOString();
  if (!indexerApiUrl) {
    return {
      available: false,
      checkedAt,
      serverTimeUnix: null as number | null,
      quarantineCount: null as number | null,
      activeQuarantines: [] as unknown[],
      bountyProvers: [] as unknown[],
      committeeDiversity: null as unknown,
    };
  }
  const [quarantineRes, leaderboardRes, diversityRes] = await Promise.all([
    fetchJsonProbe(
      "indexer_quarantine",
      joinUrl(indexerApiUrl, "/v1/index/quarantine-queue?limit=25"),
    ),
    fetchJsonProbe(
      "indexer_bounties",
      joinUrl(indexerApiUrl, "/v1/index/bounties/leaderboard?limit=10"),
    ),
    fetchJsonProbe(
      "indexer_diversity",
      joinUrl(indexerApiUrl, "/v1/index/committee-diversity?epoch=0"),
    ),
  ]);
  const qData = asRecord(quarantineRes.data);
  const qEntries = Array.isArray(qData.entries) ? qData.entries : [];
  const bData = asRecord(leaderboardRes.data);
  const bEntries = Array.isArray(bData.entries) ? bData.entries : [];
  const available =
    quarantineRes.probe.ok || leaderboardRes.probe.ok || diversityRes.probe.ok;
  return {
    available,
    checkedAt,
    serverTimeUnix: pickNumber(qData, "server_time_unix"),
    quarantineCount: quarantineRes.probe.ok ? qEntries.length : null,
    activeQuarantines: qEntries.slice(0, 5),
    bountyProvers: bEntries.slice(0, 5),
    committeeDiversity: diversityRes.probe.ok ? diversityRes.data : null,
  };
}
