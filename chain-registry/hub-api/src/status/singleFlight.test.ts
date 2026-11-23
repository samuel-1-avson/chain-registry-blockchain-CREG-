// ADR-019 Increment 1 (ECA-36): status aggregate caching.
import assert from "node:assert/strict";
import test from "node:test";
import { singleFlightCache } from "./singleFlight.js";

test("concurrent callers share one in-flight load", async () => {
  let loads = 0;
  const cache = singleFlightCache(5_000, async () => {
    loads += 1;
    await new Promise((resolve) => setTimeout(resolve, 10));
    return { n: loads };
  });
  const results = await Promise.all(Array.from({ length: 50 }, () => cache.get()));
  assert.equal(loads, 1);
  assert.ok(results.every((r) => r.n === 1));
});

test("value is reused within the TTL and refreshed after it", async () => {
  let clock = 0;
  let loads = 0;
  const cache = singleFlightCache(
    5_000,
    async () => {
      loads += 1;
      return loads;
    },
    () => clock,
  );
  assert.equal(await cache.get(), 1);
  clock = 4_999;
  assert.equal(await cache.get(), 1);
  clock = 5_000;
  assert.equal(await cache.get(), 2);
});

test("a failed load is not cached", async () => {
  let attempts = 0;
  const cache = singleFlightCache(5_000, async () => {
    attempts += 1;
    if (attempts === 1) throw new Error("upstream down");
    return "ok";
  });
  await assert.rejects(cache.get());
  assert.equal(await cache.get(), "ok");
});
