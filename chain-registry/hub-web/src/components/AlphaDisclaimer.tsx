type AlphaDisclaimerProps = {
  /** Shorter copy for inline banners; default is the full public-alpha notice. */
  compact?: boolean;
};

export function AlphaDisclaimer({ compact = false }: AlphaDisclaimerProps) {
  if (compact) {
    return (
      <p className="hub-alpha-disclaimer hub-alpha-disclaimer--compact" role="note">
        <strong>Public alpha.</strong> Not production-ready. LLM and scanner outputs
        are advisory only and do not replace validator consensus.
      </p>
    );
  }

  return (
    <aside className="hub-alpha-disclaimer" role="note" aria-label="Public alpha notice">
      <p>
        <strong>Public alpha — not mainnet or production.</strong> CREG testnet is
        open for learning and rehearsal while readiness gates (audit, validator
        fleet hardening, IPFS availability, and operational drills) remain in
        progress.
      </p>
      <p>
        <strong>LLM advisory boundary (LLM-002):</strong> deep analysis, risk
        summaries, and Lane B/C scores shown in the explorer or CLI are machine
        assistance only. They do not count toward verification quorum and must not
        be treated as a security guarantee.
      </p>
    </aside>
  );
}
