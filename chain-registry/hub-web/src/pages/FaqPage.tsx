import { AlphaDisclaimer } from "../components/AlphaDisclaimer";

const faqs = [
  [
    "What chain is this?",
    "CREG public alpha runs on Ethereum Sepolia, chain id 11155111.",
  ],
  [
    "Do I need a wallet?",
    "No for reading guides and observing. Yes for faucet, stake, publish, validator registration, and future saved progress.",
  ],
  [
    "What is the difference between pending and verified?",
    "Pending means a node accepted the package. Verified means the package is present in this testnet node chain store after the validator workflow.",
  ],
  [
    "Does the hub collect validator keys?",
    "No. Validator private keys stay in the operator environment. The hub should only ever see public wallet/session data.",
  ],
  [
    "Is this mainnet ready?",
    "No. This is public alpha. External audit, validator expansion, IPFS availability evidence, and operational rehearsals are still part of readiness.",
  ],
  [
    "Can I trust LLM or deep-analysis scores?",
    "No as a sole signal. Lane B/C outputs in the explorer and CLI are advisory machine assistance. Validator consensus and deterministic scanner findings drive verification status.",
  ],
] as const;

export function FaqPage() {
  return (
    <div className="hub-page">
      <header className="hub-page-header">
        <p className="hub-eyebrow">FAQ</p>
        <h1>Testnet questions without the fog</h1>
        <p>
          CREG testnet is useful now, but public alpha language matters. These
          answers are intentionally precise.
        </p>
      </header>

      <AlphaDisclaimer />

      <section className="hub-grid-wide">
        {faqs.map(([question, answer]) => (
          <article className="hub-card" key={question}>
            <h2>{question}</h2>
            <p>{answer}</p>
          </article>
        ))}
      </section>
    </div>
  );
}
