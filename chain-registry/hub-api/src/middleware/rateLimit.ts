import type { Context, MiddlewareHandler } from "hono";
import { resolveClientAddress } from "./clientAddress.js";

// ADR-019 Increment 1 (ECA-36): bounded limiter keyed by the trusted client
// address. The previous module-global Map was never pruned and was keyed by
// the client-written leftmost X-Forwarded-For entry, so random headers both
// bypassed the limit and grew memory without bound, while requests with no
// proxy headers all shared one "unknown" bucket (20 auth requests a minute
// locked out sign-in globally).

type Bucket = {
  count: number;
  resetAt: number;
};

/** Fixed-window counters with a hard cap on tracked keys (LRU eviction). */
export class BoundedBucketStore {
  private readonly buckets = new Map<string, Bucket>();
  private readonly maxKeys: number;

  constructor(maxKeys: number) {
    if (!Number.isFinite(maxKeys) || maxKeys < 1) {
      throw new Error("BoundedBucketStore maxKeys must be >= 1");
    }
    this.maxKeys = Math.floor(maxKeys);
  }

  get size(): number {
    return this.buckets.size;
  }

  /** Count one request for `key` and return its bucket after the increment. */
  hit(key: string, now: number, windowMs: number): Bucket {
    const existing = this.buckets.get(key);
    if (existing) {
      // Re-insert to move the key to the most-recently-used end.
      this.buckets.delete(key);
    }
    let bucket: Bucket;
    if (existing && existing.resetAt > now) {
      existing.count += 1;
      bucket = existing;
    } else {
      bucket = { count: 1, resetAt: now + windowMs };
      while (this.buckets.size >= this.maxKeys) {
        const oldest = this.buckets.keys().next().value;
        if (oldest === undefined) break;
        this.buckets.delete(oldest);
      }
    }
    this.buckets.set(key, bucket);
    return bucket;
  }
}

const DEFAULT_MAX_KEYS = (() => {
  const parsed = Number(process.env.HUB_RATE_LIMIT_MAX_KEYS ?? "10000");
  return Number.isFinite(parsed) && parsed >= 1 ? Math.floor(parsed) : 10000;
})();

const sharedStore = new BoundedBucketStore(DEFAULT_MAX_KEYS);

export type RateLimitOptions = {
  windowMs: number;
  max: number;
  keyPrefix?: string;
  /** Defaults to one process-wide store bounded by HUB_RATE_LIMIT_MAX_KEYS. */
  store?: BoundedBucketStore;
  /**
   * Refuse with 503 when no client address can be established instead of
   * sharing a bucket. Defaults to true when NODE_ENV=production or
   * HUB_REQUIRE_CLIENT_IP=true.
   */
  requireClientAddress?: boolean;
  /** Believe X-Real-IP / X-Forwarded-For from loopback/private peers. */
  trustPrivateProxies?: boolean;
};

/** Socket peer from @hono/node-server's `c.env.incoming`. */
function socketPeer(c: Context): string | undefined {
  const env = c.env as { incoming?: { socket?: { remoteAddress?: string } } } | undefined;
  return env?.incoming?.socket?.remoteAddress;
}

function defaultRequireClientAddress(): boolean {
  return (
    process.env.NODE_ENV === "production" ||
    process.env.HUB_REQUIRE_CLIENT_IP === "true"
  );
}

export function rateLimit(options: RateLimitOptions): MiddlewareHandler {
  const {
    windowMs,
    max,
    keyPrefix = "default",
    store = sharedStore,
    requireClientAddress = defaultRequireClientAddress(),
    trustPrivateProxies = process.env.HUB_TRUST_PRIVATE_PROXIES !== "false",
  } = options;

  return async (c, next) => {
    const client = resolveClientAddress(
      {
        peer: socketPeer(c),
        realIp: c.req.header("x-real-ip"),
        forwardedFor: c.req.header("x-forwarded-for"),
      },
      { trustPrivateProxies },
    );
    if (client === undefined && requireClientAddress) {
      return c.json(
        {
          error: "client_address_unavailable",
          message: "Unable to establish the client address for rate limiting.",
        },
        503,
      );
    }

    const now = Date.now();
    const bucket = store.hit(`${keyPrefix}:${client ?? "unknown"}`, now, windowMs);
    c.header("X-RateLimit-Limit", String(max));

    if (bucket.count > max) {
      const retryAfterSec = Math.max(1, Math.ceil((bucket.resetAt - now) / 1000));
      c.header("Retry-After", String(retryAfterSec));
      c.header("X-RateLimit-Remaining", "0");
      return c.json(
        {
          error: "rate_limit_exceeded",
          message: "Too many requests. Try again later.",
          retryAfterSec,
        },
        429,
      );
    }

    c.header("X-RateLimit-Remaining", String(Math.max(0, max - bucket.count)));
    await next();
  };
}
