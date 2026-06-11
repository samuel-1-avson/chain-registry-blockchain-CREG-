import { usePublicStatus } from "../hooks/usePublicStatus";
import { StatusPill } from "./StatusPill";

export function HealthBanner() {
  const status = usePublicStatus(45_000);

  return (
    <section className="hub-health" aria-live="polite">
      <div>
        <strong>Testnet status</strong>
        <p>
          {status.kind === "loading" && "Checking public API, faucet, spec, and explorer."}
          {status.kind === "error" && status.message}
          {status.kind === "ok" &&
            (status.data.status === "ok"
              ? "Core public services are reachable."
              : "One or more public services are degraded. Guides remain available.")}
        </p>
      </div>
      {status.kind === "ok" && (
        <div className="hub-health-pills">
          <StatusPill tone={status.data.status === "ok" ? "success" : "warning"}>
            {status.data.status}
          </StatusPill>
          <span>height {status.data.chain.height ?? "--"}</span>
          <span>{status.data.chain.validators ?? "--"} validators</span>
        </div>
      )}
      {status.kind === "loading" && <StatusPill tone="info">checking</StatusPill>}
      {status.kind === "error" && <StatusPill tone="error">offline</StatusPill>}
    </section>
  );
}
