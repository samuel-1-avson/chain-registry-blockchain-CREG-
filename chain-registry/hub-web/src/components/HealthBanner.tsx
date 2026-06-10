import { useEffect, useState } from "react";

type HealthResponse = {
  status: string;
  service: string;
  phase: string;
};

type HealthState =
  | { kind: "loading" }
  | { kind: "ok"; data: HealthResponse }
  | { kind: "error"; message: string };

export function HealthBanner() {
  const [health, setHealth] = useState<HealthState>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;

    async function loadHealth() {
      try {
        const response = await fetch("/api/health");
        if (!response.ok) {
          throw new Error(`HTTP ${response.status}`);
        }
        const data = (await response.json()) as HealthResponse;
        if (!cancelled) {
          setHealth({ kind: "ok", data });
        }
      } catch (error) {
        if (!cancelled) {
          const message =
            error instanceof Error ? error.message : "Hub API unreachable";
          setHealth({ kind: "error", message });
        }
      }
    }

    void loadHealth();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div style={styles.banner} aria-live="polite">
      <span style={styles.dot(health)} aria-hidden />
      <strong>Hub API</strong>
      {health.kind === "loading" && <span> Checking /api/health…</span>}
      {health.kind === "ok" && (
        <span>
          {" "}
          {health.data.status} ({health.data.service}, phase {health.data.phase})
        </span>
      )}
      {health.kind === "error" && (
        <span style={styles.error}> Offline — {health.message}</span>
      )}
    </div>
  );
}

const styles = {
  banner: {
    border: "1px solid var(--border)",
    background: "var(--bg-elevated)",
    borderRadius: "var(--radius-md)",
    padding: "0.65rem 0.9rem",
    marginBottom: "var(--space-6)",
    fontSize: "0.9rem",
    display: "flex",
    alignItems: "center",
    gap: "0.5rem",
    flexWrap: "wrap" as const,
  },
  dot: (health: HealthState) => ({
    width: 8,
    height: 8,
    borderRadius: "50%",
    background:
      health.kind === "ok"
        ? "var(--accent-success)"
        : health.kind === "error"
          ? "var(--accent-error)"
          : "var(--accent-warning)",
  }),
  error: {
    color: "var(--accent-warning)",
  },
};
