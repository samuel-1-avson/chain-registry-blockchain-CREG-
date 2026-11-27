import {
  desktopApi,
  type DesktopDoctorReport,
  type EnvironmentInfo,
  type RuntimeMode,
  type RuntimeStatus,
  type ZoneCapabilitiesV1,
} from "../api/tauri";

type Props = {
  env: EnvironmentInfo | null;
  mode: RuntimeMode;
  setMode: (mode: RuntimeMode) => void;
  composeFile: string;
  supervisorBinary: string;
  operatorKeyFile: string;
  aiConfigPath: string;
  runtimeStatus: RuntimeStatus | null;
  setRuntimeStatus: (s: RuntimeStatus) => void;
  doctor: DesktopDoctorReport | null;
  setDoctor: (d: DesktopDoctorReport) => void;
  busy: string | null;
  setBusy: (b: string | null) => void;
  errorMsg: string | null;
  setErrorMsg: (e: string | null) => void;
  hasActiveR7Node?: boolean;
};

function renderCapabilities(caps: ZoneCapabilitiesV1) {
  switch (caps.kind) {
    case "gateway":
      return (
        <dl className="kv-list">
          <dt>Public</dt>
          <dd>{caps.public_listen}</dd>
          <dt>Admin</dt>
          <dd>{caps.admin_listen}</dd>
          <dt>Rate Limit</dt>
          <dd>{caps.rate_limit_max} / {caps.rate_limit_window_secs}s</dd>
          <dt>Consensus IPC</dt>
          <dd>{caps.consensus_ipc_reachable ? "reachable" : "offline"}</dd>
        </dl>
      );
    case "consensus":
      return (
        <dl className="kv-list">
          <dt>Node ID</dt>
          <dd className="mono">{caps.node_id}</dd>
          <dt>Chain ID</dt>
          <dd>{caps.chain_id}</dd>
          <dt>Tip Height</dt>
          <dd>#{caps.tip_height}</dd>
          <dt>Hybrid PQ / Signer</dt>
          <dd>{caps.hybrid_active ? "active" : "pre-activation"} · {caps.signing_backend}</dd>
        </dl>
      );
    case "sandbox":
      return (
        <dl className="kv-list">
          <dt>Engine</dt>
          <dd>{caps.selected_engine} (avail: {caps.available_engines.join(", ") || "none"})</dd>
          <dt>Rootfs</dt>
          <dd>{caps.rootfs_configured ? "pinned" : "not configured"}</dd>
          <dt>Max Timeout</dt>
          <dd>{caps.max_timeout_secs}s</dd>
        </dl>
      );
    case "agent_runner":
      return (
        <dl className="kv-list">
          <dt>AI Mode</dt>
          <dd>{caps.ai_mode}</dd>
          <dt>Providers</dt>
          <dd>{caps.configured_providers.join(", ") || "none"} ({caps.distinct_model_families} families)</dd>
          <dt>Daily Budget</dt>
          <dd>${caps.spent_today_usd.toFixed(2)} / ${caps.daily_budget_usd.toFixed(2)}</dd>
          <dt>Sandbox IPC / IPFS</dt>
          <dd>{caps.sandbox_ipc_configured ? "ok" : "off"} / {caps.ipfs_configured ? "ok" : "off"}</dd>
        </dl>
      );
  }
}

export function ZonesView(props: Props) {
  const {
    env,
    mode,
    setMode,
    composeFile,
    supervisorBinary,
    operatorKeyFile,
    aiConfigPath,
    runtimeStatus,
    setRuntimeStatus,
    doctor,
    setDoctor,
    busy,
    setBusy,
    errorMsg,
    setErrorMsg,
    hasActiveR7Node,
  } = props;

  const isCollapsedForR7 = Boolean(hasActiveR7Node) && !runtimeStatus?.running;
  const lastError =
    runtimeStatus?.last_error && errorMsg !== runtimeStatus.last_error
      ? runtimeStatus.last_error
      : null;

  const refreshRuntime = async () => {
    setErrorMsg(null);
    setBusy("Checking runtime status...");
    try {
      const s = await desktopApi.getRuntimeStatus(mode, composeFile || undefined);
      setRuntimeStatus(s);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(null);
    }
  };

  const handleStart = async () => {
    setErrorMsg(null);
    setBusy("Starting configured runtime...");
    try {
      const s = await desktopApi.startValidator({
        mode,
        composeFile: composeFile || undefined,
        supervisorBinary: supervisorBinary || undefined,
      });
      setRuntimeStatus(s);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(null);
    }
  };

  const handleStop = async () => {
    setErrorMsg(null);
    setBusy("Stopping the four-zone runtime…");
    try {
      const s = await desktopApi.stopValidator(mode, composeFile || undefined);
      setRuntimeStatus(s);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(null);
    }
  };

  const runDoctor = async () => {
    setErrorMsg(null);
    setBusy("Probing 4 validator zones over zone-ipc...");
    try {
      const report = await desktopApi.runValidatorDoctor(
        operatorKeyFile || undefined,
        aiConfigPath || undefined,
      );
      setDoctor(report);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusy(null);
    }
  };

  const panels = (
    <div className="panel-stack">
      <section className="card">
        <div className="card-header">
          <div>
            <h2>Advanced four-zone runtime</h2>
            <p className="muted">
              Manage a configured development stack or native supervisor.
            </p>
          </div>
          <span className={`pill ${runtimeStatus?.running ? "pill-ok" : hasActiveR7Node ? "" : "pill-warn"}`}>
            {runtimeStatus?.running ? "RUNNING" : hasActiveR7Node ? "NOT USED FOR R7" : "STOPPED"}
          </span>
        </div>

        <div className="toolbar">
          <label className="inline-label">
            Mode
            <select
              value={mode}
              onChange={(e) => setMode(e.target.value as RuntimeMode)}
            >
              <option value="docker_compose">Docker Compose (validator-v2)</option>
              <option value="local_supervisor">Local Native Supervisor</option>
            </select>
          </label>
          <button type="button" className="btn btn-primary" disabled={Boolean(busy)} onClick={handleStart}>
            Start runtime
          </button>
          <button type="button" className="btn btn-danger" disabled={Boolean(busy)} onClick={handleStop}>
            Stop runtime
          </button>
          <button type="button" className="btn" disabled={Boolean(busy)} onClick={refreshRuntime}>
            Refresh Status
          </button>
        </div>

        {errorMsg && <div className="alert alert-err">{errorMsg}</div>}
        {lastError && <div className="alert alert-warn">{lastError}</div>}

        <div className="meta-strip">
          <span>Docker CLI: <strong>{env?.docker_available ? "available" : "not detected"}</strong></span>
          <span>Supervisor bin: <code className="mono">{supervisorBinary || env?.supervisor_binary || "none"}</code></span>
          {runtimeStatus?.pid != null && <span>PID: <strong>{runtimeStatus.pid}</strong></span>}
        </div>

        {runtimeStatus && runtimeStatus.containers.length > 0 && (
          <table className="table">
            <thead>
              <tr>
                <th>Container / Service</th>
                <th>State</th>
                <th>Details</th>
              </tr>
            </thead>
            <tbody>
              {runtimeStatus.containers.map((c) => (
                <tr key={c.name}>
                  <td className="mono">{c.name}</td>
                  <td>
                    <span className={`pill ${c.state === "running" ? "pill-ok" : "pill-warn"}`}>
                      {c.state}
                    </span>
                  </td>
                  <td className="muted">{c.status}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section className="card">
        <div className="card-header">
          <div>
            <h2>Four-Zone Health (`creg validator doctor`)</h2>
            <p className="muted">
              Authenticated per-link HKDF IPC diagnostics over <code>ZoneId::Operator</code>. Signing keys stay isolated inside Zone 2.
            </p>
          </div>
          <button type="button" className="btn btn-primary" disabled={Boolean(busy)} onClick={runDoctor}>
            Run 4-Zone Doctor
          </button>
        </div>

        {doctor ? (
          <>
            <div className="zone-grid">
              {doctor.zones.map((z) => (
                <article key={z.zone} className="zone-card">
                  <div className="zone-card-head">
                    <strong className="mono">{z.zone}</strong>
                    <span className={`pill ${z.healthy ? "pill-ok" : z.reachable ? "pill-warn" : "pill-err"}`}>
                      {z.healthy ? "healthy" : z.reachable ? z.status || "degraded" : "unreachable"}
                    </span>
                  </div>
                  <div className="mono small muted">{z.endpoint || "no endpoint"}</div>
                  {z.capabilities && renderCapabilities(z.capabilities.capabilities)}
                  {z.error && <div className="alert alert-err small">{z.error}</div>}
                </article>
              ))}
            </div>
            {doctor.warnings.length > 0 && (
              <div className="alert alert-warn">
                {doctor.warnings.map((w, i) => (
                  <div key={i}>{w}</div>
                ))}
              </div>
            )}
          </>
        ) : (
          <p className="muted">
            Click <strong>Run 4-Zone Doctor</strong> to probe Gateway, Consensus, Sandbox, and Agent Runner via{" "}
            <code className="mono">{operatorKeyFile || env?.default_operator_key_file || "operator-ipc-keys.json"}</code>.
          </p>
        )}
      </section>
    </div>
  );

  if (isCollapsedForR7) {
    return (
      <details className="card">
        <summary className="muted" style={{ cursor: "pointer", fontWeight: 600 }}>
          Advanced four-zone development stack (Linux KVM — not needed for your running R7 validator)
        </summary>
        {panels}
      </details>
    );
  }

  return panels;
}
