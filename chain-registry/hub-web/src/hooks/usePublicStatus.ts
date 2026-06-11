import { useEffect, useState } from "react";
import { fetchPublicStatus, type PublicStatus } from "../api/status";

type PublicStatusState =
  | { kind: "loading" }
  | { kind: "ok"; data: PublicStatus }
  | { kind: "error"; message: string };

export function usePublicStatus(refreshMs = 30_000): PublicStatusState {
  const [state, setState] = useState<PublicStatusState>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    let controller: AbortController | null = null;

    async function load() {
      controller?.abort();
      controller = new AbortController();
      try {
        const data = await fetchPublicStatus(controller.signal);
        if (!cancelled) setState({ kind: "ok", data });
      } catch (error) {
        if (!cancelled) {
          setState({
            kind: "error",
            message:
              error instanceof Error ? error.message : "Status unavailable",
          });
        }
      }
    }

    void load();
    const id = window.setInterval(() => void load(), refreshMs);
    return () => {
      cancelled = true;
      controller?.abort();
      window.clearInterval(id);
    };
  }, [refreshMs]);

  return state;
}
