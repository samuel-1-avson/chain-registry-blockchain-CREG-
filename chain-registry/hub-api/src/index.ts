import { serve } from "@hono/node-server";
import { Hono } from "hono";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";

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

type UpstreamProbe = {
  name: string;
  url: string;
  ok: boolean;
  statusCode?: number;
  latencyMs: number;
  message?: string;
};

type ProbeWithData = {
  probe: UpstreamProbe;
  data: unknown;
};

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
    phase: "2",
    dbPathConfigured: Boolean(dbPath),
  }),
);

app.get("/api/status/public", async (c) => {
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

  return c.json({
    status: criticalOk && allOk ? "ok" : "degraded",
    service: "hub-api",
    phase: "1",
    checkedAt,
    upstreams: probes,
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
  });
});

serve({ fetch: app.fetch, port }, (info) => {
  console.log(`hub-api listening on http://127.0.0.1:${info.port}`);
});

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
