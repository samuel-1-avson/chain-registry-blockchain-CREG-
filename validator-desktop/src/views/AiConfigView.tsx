import { useState } from "react";
import {
  desktopApi,
  type AiConfigCheckReportV1,
  type ProviderProbeReportV1,
} from "../api/tauri";

const SAMPLE_AI_TOML = `[agent]
mode = "triage_only"
shadow_mode = true

[providers.local_ollama]
kind = "ollama"
model = "qwen2.5:7b"
base_url = "http://127.0.0.1:11434"

[routing]
tier_fast = "local_ollama"
`;

export function AiConfigView({
  aiConfigPath,
  setAiConfigPath,
  operatorKeyFile,
}: {
  aiConfigPath: string;
  setAiConfigPath: (p: string) => void;
  operatorKeyFile: string;
}) {
  const [tomlText, setTomlText] = useState(SAMPLE_AI_TOML);
  const [checkReport, setCheckReport] = useState<AiConfigCheckReportV1 | null>(null);
  const [probeReport, setProbeReport] = useState<ProviderProbeReportV1 | null>(null);
  const [statusMsg, setStatusMsg] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const loadFromDisk = async () => {
    setBusy(true);
    setErrorMsg(null);
    setStatusMsg(null);
    try {
      const content = await desktopApi.readAiConfig(aiConfigPath);
      setTomlText(content);
      const rep = await desktopApi.checkAiConfig(aiConfigPath, content);
      setCheckReport(rep);
      setStatusMsg("Loaded and validated configuration from disk.");
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(false);
    }
  };

  const validateDraft = async () => {
    setBusy(true);
    setErrorMsg(null);
    setStatusMsg(null);
    try {
      const rep = await desktopApi.checkAiConfig(aiConfigPath || undefined, tomlText);
      setCheckReport(rep);
      setStatusMsg(rep.ok ? "Validation passed." : "Validation found blocking errors.");
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(false);
    }
  };

  const validateAndSave = async () => {
    setBusy(true);
    setErrorMsg(null);
    setStatusMsg(null);
    try {
      const rep = await desktopApi.saveAiConfig(aiConfigPath, tomlText);
      setCheckReport(rep);
      setStatusMsg("Validated and saved validator-ai.toml cleanly.");
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(false);
    }
  };

  const probeProviders = async () => {
    setBusy(true);
    setErrorMsg(null);
    setStatusMsg(null);
    try {
      const rep = await desktopApi.probeAiProviders(operatorKeyFile || undefined);
      setProbeReport(rep);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="panel-stack">
      <section className="card">
        <div className="card-header">
          <div>
            <h2>AI Provider Config (`validator-ai.toml`)</h2>
            <p className="muted">
              Validates SSRF URL policy, budgets, model-family diversity, and <code>env:/file:/vault:</code> secret refs without exposing secrets.
            </p>
          </div>
          <button type="button" className="btn btn-primary" disabled={busy} onClick={probeProviders}>
            Probe Zone 4 Providers
          </button>
        </div>

        <div className="toolbar">
          <input
            type="text"
            className="input grow mono"
            value={aiConfigPath}
            onChange={(e) => setAiConfigPath(e.target.value)}
            placeholder="Path to validator-ai.toml"
          />
          <button type="button" className="btn" disabled={busy} onClick={loadFromDisk}>
            Load File
          </button>
          <button type="button" className="btn" disabled={busy} onClick={validateDraft}>
            Validate Draft
          </button>
          <button type="button" className="btn btn-primary" disabled={busy} onClick={validateAndSave}>
            Validate &amp; Save
          </button>
        </div>

        <textarea
          className="code-editor mono"
          rows={12}
          value={tomlText}
          onChange={(e) => setTomlText(e.target.value)}
          spellCheck={false}
        />

        {statusMsg && <div className="alert alert-ok">{statusMsg}</div>}
        {errorMsg && <div className="alert alert-err">{errorMsg}</div>}

        {checkReport && (
          <div className="panel-stack" style={{ marginTop: 12 }}>
            <div className="meta-strip">
              <span className={`pill ${checkReport.ok ? "pill-ok" : "pill-err"}`}>
                {checkReport.ok ? "CHECK PASS" : "CHECK FAIL"}
              </span>
              <span>Mode: <strong>{checkReport.mode}</strong></span>
              <span>Model families: <strong>{checkReport.distinct_model_families}</strong></span>
            </div>
            {checkReport.diversity_advisory && (
              <div className="alert alert-warn">{checkReport.diversity_advisory}</div>
            )}
            {checkReport.errors.length > 0 && (
              <div className="alert alert-err">
                {checkReport.errors.map((e, i) => <div key={i}>{e}</div>)}
              </div>
            )}
            {checkReport.providers.length > 0 && (
              <table className="table">
                <thead>
                  <tr>
                    <th>Provider</th>
                    <th>Kind</th>
                    <th>Model / Family</th>
                    <th>Endpoint Class</th>
                    <th>Secret Ref</th>
                    <th>Status</th>
                  </tr>
                </thead>
                <tbody>
                  {checkReport.providers.map((p) => (
                    <tr key={p.name}>
                      <td className="mono">{p.name}</td>
                      <td>{p.kind}</td>
                      <td className="mono">{p.model} ({p.model_family})</td>
                      <td>{p.endpoint_class ?? "invalid"}</td>
                      <td>{p.secret_scheme ?? "none"}</td>
                      <td>
                        <span className={`pill ${p.url_ok && p.secret_ok && p.issues.length === 0 ? "pill-ok" : "pill-err"}`}>
                          {p.url_ok && p.secret_ok && p.issues.length === 0 ? "ok" : p.issues.join("; ")}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        )}

        {probeReport && (
          <div style={{ marginTop: 16 }}>
            <h3>Live Zone 4 Provider Probe</h3>
            <table className="table">
              <thead>
                <tr>
                  <th>Provider</th>
                  <th>Model</th>
                  <th>Endpoint</th>
                  <th>Latency</th>
                  <th>Tool JSON</th>
                  <th>Result</th>
                </tr>
              </thead>
              <tbody>
                {probeReport.providers.map((p) => (
                  <tr key={p.provider}>
                    <td className="mono">{p.provider}</td>
                    <td className="mono">{p.model} ({p.model_family})</td>
                    <td>{p.endpoint_class}</td>
                    <td>{p.latency_ms}ms</td>
                    <td>{p.valid_tool_json ? "valid" : "no"}</td>
                    <td>
                      <span className={`pill ${p.ok ? "pill-ok" : "pill-err"}`}>
                        {p.ok ? "ok" : p.error || "failed"}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}
