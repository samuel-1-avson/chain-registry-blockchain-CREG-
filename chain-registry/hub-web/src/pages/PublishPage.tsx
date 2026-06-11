import { CommandBlock } from "../components/CommandBlock";
import { StatusPill } from "../components/StatusPill";
import { EXTERNAL_LINKS } from "../config/links";

export function PublishPage() {
  return (
    <div className="hub-page">
      <header className="hub-page-header">
        <p className="hub-eyebrow">Publisher journey</p>
        <h1>Publish a signed package to CREG testnet</h1>
        <p>
          Publishers use an Ethereum wallet for Sepolia stake and an Ed25519 key
          for package signatures. The hub guides the flow; the CLI performs the
          sensitive work locally.
        </p>
        <div className="hub-actions">
          <a className="hub-button" href={`${EXTERNAL_LINKS.faucet}`}>
            Get test funds
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
      </header>

      <section className="hub-grid-wide">
        <article className="hub-card">
          <StatusPill tone="info">requirements</StatusPill>
          <h2>Before you publish</h2>
          <ul className="hub-list">
            <li>
              <span className="hub-step-title">Sepolia wallet</span>
              <span className="hub-step-body">
                Gas uses native Sepolia ETH. Publisher role operations use tCREG.
              </span>
            </li>
            <li>
              <span className="hub-step-title">1 tCREG publisher stake</span>
              <span className="hub-step-body">
                Current Sepolia Staking contract:
                <code> 0xf28C63C4Aafd27025E535Ab9ab7B4daC18C96Bc2</code>
              </span>
            </li>
            <li>
              <span className="hub-step-title">CREG CLI and IPFS</span>
              <span className="hub-step-body">
                Build or install the CLI, then publish package bytes through IPFS.
              </span>
            </li>
          </ul>
        </article>

        <article className="hub-card">
          <StatusPill tone="warning">alpha semantics</StatusPill>
          <h2>What status means</h2>
          <p>
            <strong>pending</strong> means a node accepted the package but
            validator consensus has not finalized it yet. <strong>verified</strong>
            means it is present in this testnet node chain store after the
            validator workflow. Wrong node URLs can make a package look unknown.
          </p>
        </article>
      </section>

      <section className="hub-two-column">
        <div className="hub-panel">
          <h2>Publisher checklist</h2>
          <ul className="hub-list" style={{ marginTop: "var(--space-4)" }}>
            <li>
              <span className="hub-step-title">1. Set the public node URL</span>
              <span className="hub-step-body">
                Avoid local defaults when publishing to the hosted testnet.
              </span>
            </li>
            <li>
              <span className="hub-step-title">2. Generate a package signing key</span>
              <span className="hub-step-body">
                Keep the Ed25519 key local. It is not an Ethereum wallet key.
              </span>
            </li>
            <li>
              <span className="hub-step-title">3. Stake and publish</span>
              <span className="hub-step-body">
                Stake with your EOA, then publish with the local signing key.
              </span>
            </li>
          </ul>
        </div>

        <aside className="hub-panel">
          <h2>Contracts</h2>
          <table className="hub-table" style={{ marginTop: "var(--space-3)" }}>
            <tbody>
              <tr>
                <th>Staking</th>
                <td>0xf28C63C4Aafd27025E535Ab9ab7B4daC18C96Bc2</td>
              </tr>
              <tr>
                <th>tCREG</th>
                <td>0x97c21d46B3eac604e92E907D54aA92eEc0Af550b</td>
              </tr>
              <tr>
                <th>Registry</th>
                <td>0x3aCfF05d00AC199412a94326eD8aA874aaA3596c</td>
              </tr>
            </tbody>
          </table>
        </aside>
      </section>

      <section className="hub-grid-wide">
        <CommandBlock
          label="Use the hosted public testnet API"
          command="export CREG_NODE_URL=https://api.testnet.cregnet.dev"
        />
        <CommandBlock
          label="Generate a publisher key"
          command="creg keygen publisher --out ~/.creg/publisher.key"
        />
        <CommandBlock
          label="Publish a package"
          command={"creg publish ./my-package-1.0.0.tgz \\\n  --key-file ~/.creg/publisher.key \\\n  --publisher-address 0xYourPublisherAddress \\\n  --ecosystem npm"}
        />
      </section>
    </div>
  );
}
