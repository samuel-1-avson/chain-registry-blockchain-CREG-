import { PathCard } from "../components/PathCard";
import { MetricTile } from "../components/MetricTile";
import { StatusPill } from "../components/StatusPill";
import { EXTERNAL_LINKS } from "../config/links";
import { usePublicStatus } from "../hooks/usePublicStatus";

export function HomePage() {
  const status = usePublicStatus();
  const chain = status.kind === "ok" ? status.data.chain : null;
  const loadingMetrics = status.kind === "loading" && chain == null;

  return (
    <div className="hub-page">
      <section className="hub-hero">
        <div className="hub-hero-copy">
          <p className="hub-eyebrow">
            Public alpha · Ethereum Sepolia · creg-testnet-1
          </p>
          <h1>
            Join the CREG <em>Sepolia</em> testnet
          </h1>
          <p>
            Publish signed packages, run validator infrastructure, or observe the
            network while CREG hardens its public-alpha supply-chain registry.
          </p>
          <div className="hub-actions">
            <a className="hub-button" href="#paths">
              Pick a path
            </a>
            <a
              className="hub-button-secondary"
              href={EXTERNAL_LINKS.explorer}
              target="_blank"
              rel="noreferrer"
            >
              Open explorer
            </a>
          </div>
        </div>
        <aside className="hub-hero-panel">
          <StatusPill
            tone={
              status.kind === "ok" && status.data.status === "ok"
                ? "success"
                : status.kind === "error"
                  ? "error"
                  : "warning"
            }
          >
            {status.kind === "loading"
              ? "checking"
              : status.kind === "ok"
                ? status.data.status
                : "degraded"}
          </StatusPill>
          <h2>Live testnet snapshot</h2>
          <div className="hub-metrics">
            <MetricTile
              label="Height"
              value={chain?.height}
              loading={loadingMetrics}
            />
            <MetricTile
              label="Validators"
              value={chain?.validators}
              loading={loadingMetrics}
            />
            <MetricTile
              label="Packages"
              value={chain?.packages}
              loading={loadingMetrics}
            />
            <MetricTile
              label="Finalization lag"
              value={chain?.finalizationLag}
              hint="blocks"
              loading={loadingMetrics}
            />
          </div>
        </aside>
      </section>

      <section className="hub-note">
        <strong>Public alpha:</strong> CREG testnet is live on Sepolia, but it is
        not mainnet and not a production security guarantee. Explorer deep
        analysis and CLI LLM lanes are advisory only — they do not replace
        validator consensus. Audit, fleet hardening, IPFS availability, and
        rehearsal gates remain open.
      </section>

      <section id="paths">
        <h2 className="hub-section-title">Choose your path</h2>
        <div className="hub-grid" aria-label="Contribution paths">
          <PathCard
            to="/observer"
            path="observe"
            index={1}
            title="Observe the network"
            description="Browse blocks, packages, validators, and live status without connecting a wallet."
            cta="Observe"
          />
          <PathCard
            to="/publish"
            path="publish"
            index={2}
            title="Publish packages"
            description="Stake as a publisher, sign packages with the CLI, and track pending or verified status."
            cta="Publish"
          />
          <PathCard
            to="/validate"
            path="validate"
            index={3}
            title="Run a validator"
            description="Stake, register identity, run a real sandbox, and participate in public-alpha verification."
            cta="Validate"
          />
          <PathCard
            to="/docs"
            path="docs"
            index={4}
            title="Review the docs"
            description="Read quickstarts, phase scope, validator onboarding, and operational runbooks."
            cta="Open docs"
          />
        </div>
      </section>
    </div>
  );
}
