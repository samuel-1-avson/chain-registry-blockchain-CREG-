import { useState } from "react";
import {
  desktopApi,
  type CommitteeDiversityV1,
  type PackageEvidenceBundle,
  type QuarantineQueueV1,
} from "../api/tauri";
import { UntrustedText } from "../components/UntrustedText";

function formatRemaining(expiresAt: number, serverTime: number): string {
  const diff = expiresAt - serverTime;
  if (diff <= 0) return "Expired -> fails closed to REVOKED";
  const h = Math.floor(diff / 3600);
  const m = Math.floor((diff % 3600) / 60);
  return `${h}h ${m}m remaining`;
}

export function EvidenceView({
  nodeUrl,
  indexerUrl,
}: {
  nodeUrl: string;
  indexerUrl: string;
}) {
  const [queue, setQueue] = useState<QuarantineQueueV1 | null>(null);
  const [canonicalInput, setCanonicalInput] = useState("npm:left-pad@1.3.0");
  const [evidence, setEvidence] = useState<PackageEvidenceBundle | null>(null);
  const [epoch, setEpoch] = useState(0);
  const [diversity, setDiversity] = useState<CommitteeDiversityV1 | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const loadQueue = async () => {
    setBusy(true);
    setErrorMsg(null);
    try {
      const q = await desktopApi.fetchQuarantineQueue(nodeUrl, indexerUrl || undefined);
      setQueue(q);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(false);
    }
  };

  const inspectPackage = async () => {
    setBusy(true);
    setErrorMsg(null);
    try {
      const res = await desktopApi.fetchPackageEvidence(
        nodeUrl,
        indexerUrl || undefined,
        canonicalInput,
      );
      setEvidence(res);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(false);
    }
  };

  const loadDiversity = async () => {
    setBusy(true);
    setErrorMsg(null);
    try {
      const res = await desktopApi.fetchCommitteeDiversity(indexerUrl || nodeUrl, epoch);
      setDiversity(res);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel-stack">
      {errorMsg && <div className="alert alert-err">{errorMsg}</div>}

      <section className="card">
        <div className="card-header">
          <div>
            <h2>72h Quarantine Escalation Queue</h2>
            <p className="muted">Fails closed to REVOKED at expiry unless released by committee quorum.</p>
          </div>
          <button type="button" className="btn btn-primary" disabled={busy} onClick={loadQueue}>
            Load Quarantine Queue
          </button>
        </div>
        {queue ? (
          queue.entries.length === 0 ? (
            <p className="muted">No packages currently quarantined.</p>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>Package</th>
                  <th>Category</th>
                  <th>Remaining (Server Clock)</th>
                  <th>Committee</th>
                </tr>
              </thead>
              <tbody>
                {queue.entries.map((entry, idx) => (
                  <tr key={`${entry.canonical}-${idx}`}>
                    <td><UntrustedText value={entry.canonical} maxGraphemes={96} /></td>
                    <td><span className="pill pill-warn">{entry.reason_code}</span></td>
                    <td className="mono">{formatRemaining(entry.expires_at_unix, queue.server_time_unix)}</td>
                    <td className="mono small">
                      +{entry.committee.approvals} / ?{entry.committee.quarantine_reviews} / -{entry.committee.rejections}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )
        ) : (
          <p className="muted">Click Load Quarantine Queue to fetch live Class C holds.</p>
        )}
      </section>

      <section className="card">
        <div className="card-header">
          <h2>Package v4 Evidence &amp; Trigger Witness Inspector</h2>
        </div>
        <div className="toolbar">
          <input
            type="text"
            className="input grow mono"
            value={canonicalInput}
            onChange={(e) => setCanonicalInput(e.target.value)}
            placeholder="ecosystem:name@version (e.g. npm:left-pad@1.3.0)"
          />
          <button type="button" className="btn btn-primary" disabled={busy} onClick={inspectPackage}>
            Inspect Evidence
          </button>
        </div>

        {evidence && (
          <div className="panel-stack" style={{ marginTop: 12 }}>
            {evidence.warnings.length > 0 && (
              <div className="alert alert-warn">
                {evidence.warnings.map((w, i) => <div key={i}>{w}</div>)}
              </div>
            )}
            {evidence.status_v4 && (
              <div className="meta-strip">
                <span className="pill">{evidence.status_v4.status.state}</span>
                <span>Quorum: <strong>{evidence.status_v4.quorum}</strong></span>
                {evidence.status_v4.status.authority && (
                  <span>Authority: <code className="mono">{evidence.status_v4.status.authority}</code></span>
                )}
              </div>
            )}
            {evidence.witnesses && evidence.witnesses.witnesses.length > 0 && (
              <div className="panel-stack">
                {evidence.witnesses.witnesses.map((w) => (
                  <article key={w.digest} className="zone-card">
                    <div className="zone-card-head">
                      <span className="pill pill-warn">{w.verification.kind}</span>
                      <span className="mono small">creg evidence verify-witness {w.digest}</span>
                    </div>
                    <div className="small">
                      <strong>Violations:</strong> {w.violations.map((v) => v.title).join(", ") || "none"}
                    </div>
                    {w.env_overrides.map(([k, v], i) => (
                      <div key={i} className="small">
                        <UntrustedText value={k} maxGraphemes={48} label="env" /> ={" "}
                        <UntrustedText value={v} maxGraphemes={128} expandable label="val" />
                      </div>
                    ))}
                  </article>
                ))}
              </div>
            )}
          </div>
        )}
      </section>

      <section className="card">
        <div className="card-header">
          <h2>Committee Diversity Telemetry</h2>
          <div className="toolbar">
            <input
              type="number"
              className="input"
              style={{ width: 90 }}
              min={0}
              value={epoch}
              onChange={(e) => setEpoch(Math.max(0, Number(e.target.value) || 0))}
            />
            <button type="button" className="btn" disabled={busy} onClick={loadDiversity}>
              Load Epoch
            </button>
          </div>
        </div>
        {diversity && (
          <div className="meta-strip">
            <span>Observations: <strong>{diversity.observations}</strong></span>
            <span>Hybrid/Classical: <strong>{diversity.hybrid_signers}/{diversity.classical_signers}</strong></span>
            <span>Families: {diversity.model_families.map((m) => `${m.model_family}(${m.votes})`).join(", ") || "none"}</span>
          </div>
        )}
      </section>
    </div>
  );
}
