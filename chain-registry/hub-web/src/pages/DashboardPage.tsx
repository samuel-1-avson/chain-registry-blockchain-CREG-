import { MetricTile } from "../components/MetricTile";
import { StatusPill } from "../components/StatusPill";
import { EXTERNAL_LINKS, SEPOLIA_CHAIN_ID } from "../config/links";
import { usePublicStatus } from "../hooks/usePublicStatus";
import { useWallet } from "../wallet/WalletProvider";

export function DashboardPage() {
  const wallet = useWallet();
  const status = usePublicStatus();
  const address = wallet.address;
  const faucetUrl = address
    ? `${EXTERNAL_LINKS.faucet}?address=${encodeURIComponent(address)}`
    : EXTERNAL_LINKS.faucet;

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

      <section className="hub-note">
        <strong>Coming next:</strong> sign-in with Ethereum, saved journey
        progress, wallet-specific balances, publisher stake hints, validator
        profile checks, and chain-verified quest completion.
      </section>
    </div>
  );
}
