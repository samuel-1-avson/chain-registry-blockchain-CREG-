import { EXTERNAL_LINKS } from "../config/links";

export function DocsPage() {
  const docs = [
    ["Public testnet quickstart", EXTERNAL_LINKS.docs, "Publisher, developer, and validator first steps"],
    [
      "Phase scope and limits",
      "https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/TESTNET_PHASE_SCOPE.md",
      "What public alpha means today",
    ],
    [
      "Validator onboarding checklist",
      "https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/VALIDATOR_ONBOARDING_CHECKLIST.md",
      "Admission policy, stake, sandbox, and health checks",
    ],
    [
      "Incident response runbook",
      "https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/INCIDENT_RESPONSE_RUNBOOK.md",
      "Operational response for alpha incidents",
    ],
    [
      "Operator runbook",
      "https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/chain-registry/testnet/OPERATOR.md",
      "Fleet, sandbox, IPFS, distribution, and hosting operations",
    ],
  ] as const;

  return (
    <div className="hub-page">
      <header className="hub-page-header">
        <p className="hub-eyebrow">Documentation</p>
        <h1>Use the right runbook for the job</h1>
        <p>
          The hub curates the public-alpha docs so users do not need to discover
          them from scattered repository paths.
        </p>
      </header>

      <section className="hub-grid-wide">
        {docs.map(([title, href, description]) => (
          <article className="hub-card" key={title}>
            <h2>{title}</h2>
            <p>{description}</p>
            <a
              className="hub-button-secondary"
              href={href}
              target="_blank"
              rel="noreferrer"
            >
              Open document
            </a>
          </article>
        ))}
      </section>
    </div>
  );
}
