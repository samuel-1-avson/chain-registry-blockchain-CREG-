// ADR-019 Increment 1 (ECA-36): `/api/status/public` fanned out six uncached
// upstream fetches per request, so request volume translated one-to-six into
// upstream load. `singleFlightCache` serves one result for `ttlMs` and makes
// concurrent callers during a refresh share the same in-flight promise.

export type SingleFlightCache<T> = {
  get(): Promise<T>;
  /** Drop the cached value (tests / forced refresh). */
  clear(): void;
};

export function singleFlightCache<T>(
  ttlMs: number,
  load: () => Promise<T>,
  now: () => number = Date.now,
): SingleFlightCache<T> {
  let cached: { at: number; value: T } | undefined;
  let inflight: Promise<T> | undefined;

  return {
    get(): Promise<T> {
      if (cached && now() - cached.at < ttlMs) {
        return Promise.resolve(cached.value);
      }
      if (!inflight) {
        inflight = load()
          .then((value) => {
            cached = { at: now(), value };
            return value;
          })
          .finally(() => {
            inflight = undefined;
          });
      }
      return inflight;
    },
    clear(): void {
      cached = undefined;
    },
  };
}
