import type { Context, MiddlewareHandler } from "hono";

type Bucket = {
  count: number;
  resetAt: number;
};

const buckets = new Map<string, Bucket>();

export type RateLimitOptions = {
  windowMs: number;
  max: number;
  keyPrefix?: string;
};

function clientIp(c: Context): string {
  const forwarded = c.req.header("x-forwarded-for");
  if (forwarded) {
    return forwarded.split(",")[0]?.trim() || "unknown";
  }
  return c.req.header("x-real-ip") ?? "unknown";
}

export function rateLimit(options: RateLimitOptions): MiddlewareHandler {
  const { windowMs, max, keyPrefix = "default" } = options;

  return async (c, next) => {
    const key = `${keyPrefix}:${clientIp(c)}`;
    const now = Date.now();
    const existing = buckets.get(key);

    if (!existing || existing.resetAt <= now) {
      buckets.set(key, { count: 1, resetAt: now + windowMs });
      c.header("X-RateLimit-Limit", String(max));
      c.header("X-RateLimit-Remaining", String(max - 1));
      await next();
      return;
    }

    if (existing.count >= max) {
      const retryAfterSec = Math.ceil((existing.resetAt - now) / 1000);
      c.header("Retry-After", String(retryAfterSec));
      c.header("X-RateLimit-Limit", String(max));
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

    existing.count += 1;
    c.header("X-RateLimit-Limit", String(max));
    c.header("X-RateLimit-Remaining", String(Math.max(0, max - existing.count)));
    await next();
  };
}
