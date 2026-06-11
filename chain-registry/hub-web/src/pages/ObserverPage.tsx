import { MetricTile } from "../components/MetricTile";
import { StatusPill } from "../components/StatusPill";
import { EXTERNAL_LINKS } from "../config/links";
import { usePublicStatus } from "../hooks/usePublicStatus";

export function ObserverPage() {
  const status = usePublicStatus();
  const chain = status.kind === "ok" ? status.data.chain : null;
  const loadingMetrics = status.kind === "loading" && chain == null;

  return (
    <div className="hub-page">
      <header className="hub-page-header">
        <p className="hub-eyebrow">Observer path</p>
        <h1>Explore CREG testnet without committing stake</h1>
        <p>
          Observers can learn the network, inspect package activity, follow
          validator status, and read the signed chain spec without connecting a
          wallet.
        </p>
        <div className="hub-actions">
          <a
            className="hub-button"
            href={EXTERNAL_LINKS.explorer}
            target="_blank"
            rel="noreferrer"
          >
            Open explorer
          </a>
          <a
            className="hub-button-secondary"
            href={EXTERNAL_LINKS.spec}
            target="_blank"
            rel="noreferrer"
          >
            View chain spec
          </a>
        </div>
      </header>

      <section className="hub-metrics">
        <MetricTile label="Height" value={chain?.height} loading={loadingMetrics} />
        <MetricTile label="Validators" value={chain?.validators} loading={loadingMetrics} />
        <MetricTile label="Packages" value={chain?.packages} loading={loadingMetrics} />
        <MetricTile
          label="Lag"
          value={chain?.finalizationLag}
          hint="blocks"
          loading={loadingMetrics}
        />
      </section>

      <section className="hub-grid-wide">
        <article className="hub-card">
          <StatusPill tone="info">start here</StatusPill>
          <h2>What to inspect first</h2>
          <ul className="hub-list">
            <li>
              <span className="hub-step-title">Network dashboard</span>
              <span className="hub-step-body">
                Check live height, validators, packages, bridge status, and
                finalization lag.
              </span>
            </li>
            <li>
              <span className="hub-step-title">Package list</span>
              <span className="hub-step-body">
                Look at pending, verified, and revoked package records.
              </span>
            </li>
            <li>
              <span className="hub-step-title">Validator pages</span>
              <span className="hub-step-body">
                Review validator registration and active-set information.
              </span>
            </li>
          </ul>
        </article>

        <article className="hub-card">
          <StatusPill tone="muted">next step</StatusPill>
          <h2>Ready to participate?</h2>
          <p>
            Choose Publish if you want to ship signed packages. Choose Validate
            if you want to operate infrastructure and join verification rounds.
          </p>
          <div className="hub-actions">
            <a className="hub-button-secondary" href="/publish">
              Publish path
            </a>
            <a className="hub-button-secondary" href="/validate">
              Validate path
            </a>
          </div>
        </article>
      </section>
    </div>
  );
}
