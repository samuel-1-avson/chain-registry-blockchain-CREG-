import { CommandBlock } from "../components/CommandBlock";
import { StatusPill } from "../components/StatusPill";
import { EXTERNAL_LINKS } from "../config/links";

export function ValidatePage() {
  return (
    <div className="hub-page">
      <header className="hub-page-header">
        <p className="hub-eyebrow">Validator journey</p>
        <h1>Prepare to run a public-alpha validator</h1>
        <p>
          Validators stake on Sepolia, register identity, run the node with a
          real sandbox engine, and participate in package verification. The hub
          never asks for validator private keys.
        </p>
        <div className="hub-actions">
          <a className="hub-button" href={EXTERNAL_LINKS.docs}>
            Read quickstart
          </a>
          <a className="hub-button-secondary" href="/dashboard">
            Check wallet readiness
          </a>
        </div>
      </header>

      <section className="hub-note">
        <strong>Key safety rule:</strong> public validators must not run with
        <code> CREG_DEV_SANDBOX=true</code>. Votes from dev-bypass profiles are
        advisory and should not count toward public verification quorum.
      </section>

      <section className="hub-grid-wide">
        <article className="hub-card">
          <StatusPill tone="info">admission</StatusPill>
          <h2>Public-alpha requirements</h2>
          <ul className="hub-list">
            <li>
              <span className="hub-step-title">100 tCREG validator stake</span>
              <span className="hub-step-body">
                Stake through the Sepolia Staking contract before admission.
              </span>
            </li>
            <li>
              <span className="hub-step-title">Dual identity</span>
              <span className="hub-step-body">
                Use an Ethereum EOA for stake and an Ed25519 key for consensus
                identity. Keep both local and never paste keys into the hub.
              </span>
            </li>
            <li>
              <span className="hub-step-title">Real sandbox and scanner profile</span>
              <span className="hub-step-body">
                Run nsjail, gVisor, or Docker sandboxing with the fleet scanner
                profile and evidence digest.
              </span>
            </li>
          </ul>
        </article>

        <article className="hub-card">
          <StatusPill tone="warning">operator path</StatusPill>
          <h2>Validator onboarding is coordinated</h2>
          <p>
            Public alpha validator admission requires stake, identity
            registration, consensus approval, and health evidence. Use the
            operator checklist before requesting admission.
          </p>
          <a className="hub-button-secondary" href={EXTERNAL_LINKS.docs}>
            Open operator docs
          </a>
        </article>
      </section>

      <section className="hub-two-column">
        <div className="hub-panel">
          <h2>Validator checklist</h2>
          <ul className="hub-list" style={{ marginTop: "var(--space-4)" }}>
            <li>
              <span className="hub-step-title">1. Prepare a Linux validator host</span>
              <span className="hub-step-body">
                Minimum alpha target: 4 vCPU, 8 GB RAM, 50 GB SSD, Docker or
                release binary, archive-capable Sepolia RPC.
              </span>
            </li>
            <li>
              <span className="hub-step-title">2. Stake and register identity</span>
              <span className="hub-step-body">
                Bind EVM address, node ID, and Ed25519 pubkey through the node API.
              </span>
            </li>
            <li>
              <span className="hub-step-title">3. Prove runtime health</span>
              <span className="hub-step-body">
                Verify sandbox engine, P2P connectivity, validator-set sync, and
                vote participation.
              </span>
            </li>
          </ul>
        </div>

        <aside className="hub-panel">
          <h2>Do not paste keys</h2>
          <p style={{ marginTop: "var(--space-3)" }}>
            The browser should only know your public wallet address. Validator
            Ed25519 keys, EOA private keys, and operator API keys remain in your
            local operator environment.
          </p>
        </aside>
      </section>

      <section className="hub-grid-wide">
        <CommandBlock
          label="Stake as validator"
          command={"creg stake --amount 100 --role validator \\\n  --key ~/.creg/validator-eoa.key \\\n  --rpc-url \"$SEPOLIA_RPC_URL\""}
        />
        <CommandBlock
          label="Check runtime sandbox"
          command={"curl -s \"$CREG_NODE_URL/v1/runtime/config\" | jq '{sandbox_engine, sandbox_dev_bypass}'"}
        />
      </section>
    </div>
  );
}
