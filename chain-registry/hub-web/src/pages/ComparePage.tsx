import { Link } from "react-router-dom";
import { StatusPill } from "../components/StatusPill";

export function ComparePage() {
  const rows = [
    {
      path: "Observer",
      wallet: "Optional",
      stake: "None",
      skills: "Explorer, docs, status reading",
      action: "/observer",
    },
    {
      path: "Publisher",
      wallet: "Required",
      stake: "1 tCREG",
      skills: "CLI, IPFS, package signing",
      action: "/publish",
    },
    {
      path: "Validator",
      wallet: "Required",
      stake: "100 tCREG",
      skills: "Linux ops, RPC, monitoring, sandboxing",
      action: "/validate",
    },
    {
      path: "Security",
      wallet: "Optional",
      stake: "None",
      skills: "Audit review, fixture testing, issue reports",
      action: "/docs",
    },
  ];

  return (
    <div className="hub-page">
      <header className="hub-page-header">
        <p className="hub-eyebrow">Compare paths</p>
        <h1>Choose the right testnet role</h1>
        <p>
          Start as an observer if you are new. Move into publishing when you
          want to test package workflows, or validating when you are ready to run
          infrastructure.
        </p>
      </header>

      <section className="hub-table-wrap">
        <table className="hub-table">
          <thead>
            <tr>
              <th>Path</th>
              <th>Wallet</th>
              <th>Stake</th>
              <th>Skills</th>
              <th>Start</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.path}>
                <td>
                  <StatusPill tone={row.path === "Validator" ? "warning" : "info"}>
                    {row.path}
                  </StatusPill>
                </td>
                <td>{row.wallet}</td>
                <td>{row.stake}</td>
                <td>{row.skills}</td>
                <td>
                  <Link className="hub-button-secondary" to={row.action}>
                    Open
                  </Link>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
}
