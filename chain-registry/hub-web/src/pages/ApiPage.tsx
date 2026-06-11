import { CommandBlock } from "../components/CommandBlock";
import { EXTERNAL_LINKS } from "../config/links";

export function ApiPage() {
  return (
    <div className="hub-page">
      <header className="hub-page-header">
        <p className="hub-eyebrow">API reference</p>
        <h1>Build against the public CREG node API</h1>
        <p>
          Use the public read routes for chain stats, packages, blocks,
          validators, addresses, and bridge status. Publisher and validator
          write routes require the correct credentials.
        </p>
        <div className="hub-actions">
          <a
            className="hub-button"
            href={EXTERNAL_LINKS.apiDocs}
            target="_blank"
            rel="noreferrer"
          >
            Open Swagger UI
          </a>
          <a className="hub-button-secondary" href={EXTERNAL_LINKS.api}>
            API base
          </a>
        </div>
      </header>

      <section className="hub-grid-wide">
        <CommandBlock
          label="Health"
          command="curl -s https://api.testnet.cregnet.dev/v1/public/health | jq ."
        />
        <CommandBlock
          label="Chain stats"
          command="curl -s https://api.testnet.cregnet.dev/v1/public/chain/stats | jq ."
        />
        <CommandBlock
          label="Packages"
          command="curl -s 'https://api.testnet.cregnet.dev/v1/public/packages?limit=10' | jq ."
        />
      </section>
    </div>
  );
}
