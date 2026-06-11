import { MetricTile } from "../components/MetricTile";
import { StatusPill } from "../components/StatusPill";
import { usePublicStatus } from "../hooks/usePublicStatus";

const labels: Record<string, string> = {
  node_api: "Node API",
  node_chain_stats: "Chain stats",
  faucet: "Faucet",
  faucet_stats: "Faucet stats",
  chain_spec: "Chain spec",
  explorer: "Explorer",
};

export function NetworkPage() {
  const status = usePublicStatus(20_000);

  if (status.kind === "loading") {
    return (
      <div className="hub-page">
        <header className="hub-page-header">
          <p className="hub-eyebrow">Network status</p>
          <h1>Checking public testnet services</h1>
          <p>Loading API, faucet, explorer, and spec server status.</p>
        </header>
      </div>
    );
  }

  if (status.kind === "error") {
    return (
      <div className="hub-page">
        <header className="hub-page-header">
          <p className="hub-eyebrow">Network status</p>
          <h1>Status endpoint unavailable</h1>
          <p>{status.message}</p>
        </header>
      </div>
    );
  }

  const { data } = status;

  return (
    <div className="hub-page">
      <header className="hub-page-header">
        <p className="hub-eyebrow">Network status</p>
        <h1>Public testnet service health</h1>
        <p>
          This page aggregates the hub API, CREG node API, faucet, explorer, and
          signed chain-spec server. It is designed to degrade gracefully when an
          upstream is offline. Green status here does not imply mainnet readiness
          or that LLM advisory outputs are authoritative.
        </p>
      </header>

      <section className="hub-metrics">
        <MetricTile label="Height" value={data.chain.height} />
        <MetricTile label="Finalized" value={data.chain.finalizedHeight} />
        <MetricTile label="Validators" value={data.chain.validators} />
        <MetricTile label="Packages" value={data.chain.packages} />
        <MetricTile
          label="Faucet drips"
          value={data.faucet.totalDrips}
          hint={
            data.faucet.cooldownSeconds
              ? `${data.faucet.cooldownSeconds}s cooldown`
              : undefined
          }
        />
      </section>

      <section className="hub-panel">
        <div className="hub-actions" style={{ marginTop: 0 }}>
          <StatusPill tone={data.status === "ok" ? "success" : "warning"}>
            {data.status}
          </StatusPill>
          <span style={{ color: "var(--text-tertiary)", fontSize: "0.9rem" }}>
            Checked {new Date(data.checkedAt).toLocaleString()}
          </span>
        </div>
        <table className="hub-table" style={{ marginTop: "var(--space-4)" }}>
          <thead>
            <tr>
              <th>Service</th>
              <th>Status</th>
              <th>Latency</th>
              <th>URL</th>
            </tr>
          </thead>
          <tbody>
            {data.upstreams.map((probe) => (
              <tr key={probe.name}>
                <td>{labels[probe.name] ?? probe.name}</td>
                <td>
                  <StatusPill tone={probe.ok ? "success" : "error"}>
                    {probe.ok ? "online" : "degraded"}
                  </StatusPill>
                </td>
                <td>{probe.latencyMs}ms</td>
                <td>
                  <a href={probe.url} target="_blank" rel="noreferrer">
                    {probe.url}
                  </a>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="hub-grid-wide">
        <article className="hub-card">
          <h2>Faucet readiness</h2>
          <p>
            tCREG reserve: {data.faucet.tokenReserve ?? "--"} · ETH reserve:{" "}
            {data.faucet.nativeReserve ?? "--"}
          </p>
          <div className="hub-actions">
            <StatusPill
              tone={data.faucet.tokenDripsAvailable ? "success" : "warning"}
            >
              tCREG drip
            </StatusPill>
            <StatusPill
              tone={data.faucet.nativeDripsAvailable ? "success" : "warning"}
            >
              ETH drip
            </StatusPill>
          </div>
        </article>

        <article className="hub-card">
          <h2>Alpha readiness lens</h2>
          <p>
            Service health is not the same as production readiness. Public alpha
            still depends on sandbox evidence, IPFS availability, validator
            diversity, and SEC-401 audit progress.
          </p>
        </article>
      </section>
    </div>
  );
}
