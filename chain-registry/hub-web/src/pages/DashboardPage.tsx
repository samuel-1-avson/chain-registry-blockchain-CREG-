import { useQuery } from "@tanstack/react-query";
import { MetricTile } from "../components/MetricTile";
import { StatusPill } from "../components/StatusPill";
import { UntrustedText } from "../components/UntrustedText";
import { fetchIndexSummary } from "../api/status";
import { EXTERNAL_LINKS, SEPOLIA_CHAIN_ID } from "../config/links";
import { usePublicStatus } from "../hooks/usePublicStatus";
import { useWallet } from "../wallet/WalletProvider";

export function DashboardPage() {
  const wallet = useWallet();
  const status = usePublicStatus();
  const indexSummary = useQuery({
    queryKey: ["index-summary"],
    queryFn: ({ signal }) => fetchIndexSummary(signal),
    staleTime: 30_000,
    refetchInterval: 30_000,
    retry: 1,
  });
  const address = wallet.address;
  const faucetUrl = address
    ? `${EXTERNAL_LINKS.faucet}?address=${encodeURIComponent(address)}`
    : EXTERNAL_LINKS.faucet;
  const idx = indexSummary.data;

  return (
    <div className="hub-page">
      <header className="hub-page-header">
        <p className="hub-eyebrow">Dashboard preview</p>
        <h1>Your testnet readiness</h1>
        <p>
          This first dashboard slice checks wallet connection and network
          context. SIWE sessions and saved quest progress come next.
        </p>
      </header>

      <section className="hub-grid-wide">
        <article className="hub-card">
          <StatusPill tone={wallet.connected ? "success" : "warning"}>
            {wallet.connected ? "wallet connected" : "wallet needed"}
          </StatusPill>
          <h2>Wallet</h2>
          <p>
            {address
              ? `${address.slice(0, 6)}...${address.slice(-4)}`
              : "Connect a wallet to preview Sepolia readiness and faucet handoffs."}
          </p>
          <div className="hub-metrics">
            <MetricTile
              label="Chain"
              value={wallet.chainId ?? "--"}
              hint={`target ${SEPOLIA_CHAIN_ID}`}
            />
            <MetricTile
              label="Sepolia"
              value={wallet.onSepolia ? "yes" : wallet.connected ? "no" : "--"}
            />
          </div>
        </article>

        <article className="hub-card">
          <StatusPill
            tone={
              status.kind === "ok" && status.data.status === "ok"
                ? "success"
                : "warning"
            }
          >
            suite status
          </StatusPill>
          <h2>Recommended next action</h2>
          <p>
            {!wallet.connected
              ? "Connect a wallet, then use the faucet for Sepolia ETH and tCREG."
              : wallet.onSepolia
                ? "Use the faucet, then continue with Publish or Validate."
                : "Switch your wallet to Sepolia before using on-chain steps."}
          </p>
          <div className="hub-actions">
            <a className="hub-button" href={faucetUrl}>
              Open faucet
            </a>
            <a className="hub-button-secondary" href="/publish">
              Publish path
            </a>
            <a className="hub-button-secondary" href="/validate">
              Validate path
            </a>
          </div>
        </article>
      </section>

      <section className="hub-card" style={{ marginTop: "1rem" }}>
        <StatusPill tone={idx?.available ? "success" : "warning"}>
          {idx?.available ? "indexer live" : "indexer unavailable"}
        </StatusPill>
        <h2>Registry v4 Telemetry</h2>
        {idx?.available ? (
          <div style={{ display: "grid", gap: "0.75rem" }}>
            <div className="hub-metrics">
              <MetricTile
                label="72h Quarantines"
                value={idx.quarantineCount ?? "--"}
                hint="fails closed to REVOKED"
              />
              <MetricTile
                label="Bounty Provers"
                value={idx.bountyProvers.length}
                hint="25% paid · 75% reserved, not distributed"
              />
              <MetricTile
                label="Epoch 0 Hybrid / Classical"
                value={
                  idx.committeeDiversity
                    ? `${idx.committeeDiversity.hybrid_signers}/${idx.committeeDiversity.classical_signers}`
                    : "--"
                }
                hint="chain-derived signers"
              />
            </div>
            {idx.activeQuarantines.length > 0 && (
              <div>
                <strong>Active quarantine holds:</strong>
                <ul>
                  {idx.activeQuarantines.slice(0, 3).map((q, i) => (
                    <li key={`${q.canonical}-${i}`}>
                      <UntrustedText value={q.canonical} maxGraphemes={96} />
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {idx.committeeDiversity && idx.committeeDiversity.model_families.length > 0 && (
              <div>
                <strong>Committee model families: </strong>
                {idx.committeeDiversity.model_families.slice(0, 5).map((mf, i) => (
                  <span key={`${mf.model_family}-${i}`} style={{ marginRight: "0.75rem" }}>
                    <UntrustedText value={mf.model_family} maxGraphemes={48} /> ({mf.votes})
                  </span>
                ))}
              </div>
            )}
            <div className="hub-actions">
              <a className="hub-button-secondary" href={`${EXTERNAL_LINKS.explorer}/quarantine`}>
                Quarantine queue
              </a>
              <a className="hub-button-secondary" href={`${EXTERNAL_LINKS.explorer}/bounties`}>
                Bounty leaderboard
              </a>
            </div>
          </div>
        ) : (
          <p>
            Public indexer aggregates are unavailable in this environment. Visit{" "}
            <a href={EXTERNAL_LINKS.explorer}>Explorer</a> for direct chain views.
          </p>
        )}
      </section>

      <section className="hub-note">
        <strong>Coming next:</strong> sign-in with Ethereum, saved journey
        progress, wallet-specific balances, publisher stake hints, validator
        profile checks, and chain-verified quest completion.
      </section>
    </div>
  );
}
