import { useCallback, useEffect, useRef, useState } from "react";
import {
  desktopApi,
  isTauriRuntime,
  type DesktopDoctorReport,
  type EnvironmentInfo,
  type R7RuntimeStatus,
  type RuntimeMode,
  type RuntimeStatus,
  type SseEnvelopeV2,
} from "./api/tauri";
import { ZonesView } from "./views/ZonesView";
import { AiConfigView } from "./views/AiConfigView";
import { LogsStreamView } from "./views/LogsStreamView";
import { EvidenceView } from "./views/EvidenceView";

type TabId = "zones" | "ai" | "logs" | "evidence" | "settings";

function r7NodeLabel(node: R7RuntimeStatus): string {
  if (node.health) {
    const role = node.health.is_validator ? "Validator" : "Observer";
    return `${role} ${node.health.synced ? "synced" : "syncing"}`;
  }
  switch (node.container_state) {
    case "running": return "Health check unavailable";
    case "created": return "Created";
    case "restarting": return "Restarting";
    case "paused": return "Paused";
    default: return "Stopped";
  }
}

export function App() {
  const [tab, setTab] = useState<TabId>("zones");
  const [env, setEnv] = useState<EnvironmentInfo | null>(null);
  const [mode, setMode] = useState<RuntimeMode>("docker_compose");
  const [composeFile, setComposeFile] = useState("");
  const [supervisorBinary, setSupervisorBinary] = useState("");
  const [operatorKeyFile, setOperatorKeyFile] = useState("");
  const [aiConfigPath, setAiConfigPath] = useState("");
  const [nodeUrl, setNodeUrl] = useState("http://127.0.0.1:8080");
  const [indexerUrl, setIndexerUrl] = useState("http://127.0.0.1:8090");
  const [sseUrl, setSseUrl] = useState("http://127.0.0.1:8090/v1/index/events");
  const [runtimeStatus, setRuntimeStatus] = useState<RuntimeStatus | null>(null);
  const [r7Runtimes, setR7Runtimes] = useState<R7RuntimeStatus[] | null>(null);
  const [r7Error, setR7Error] = useState<string | null>(null);
  const r7CheckInFlight = useRef(false);
  const [doctor, setDoctor] = useState<DesktopDoctorReport | null>(null);
  const [overview, setOverview] = useState<string | null>(null);
  const [events, setEvents] = useState<SseEnvelopeV2[]>([]);
  const [sseState, setSseState] = useState("idle");
  const [busy, setBusy] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [onboardingStatus, setOnboardingStatus] = useState<string | null>(null);

  const refreshR7 = useCallback(async () => {
    if (!isTauriRuntime() || r7CheckInFlight.current) return;
    r7CheckInFlight.current = true;
    try {
      setR7Runtimes(await desktopApi.getR7RuntimeStatus());
      setR7Error(null);
    } catch (error) {
      setR7Runtimes(null);
      setR7Error(String(error));
    } finally {
      r7CheckInFlight.current = false;
    }
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    void refreshR7();
    const timer = window.setInterval(refreshR7, 15000);
    return () => window.clearInterval(timer);
  }, [refreshR7]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    desktopApi.detectEnvironment().then((info) => {
      setEnv(info);
      setComposeFile(info.compose_file ?? "");
      setSupervisorBinary(info.supervisor_binary ?? "");
      setOperatorKeyFile(info.default_operator_key_file);
      setAiConfigPath(info.default_ai_config_path);
      desktopApi
        .getRuntimeStatus(
          info.compose_file ? "docker_compose" : "local_supervisor",
          info.compose_file ?? undefined,
        )
        .then(setRuntimeStatus)
        .catch(() => {});
    });

    const unlistenPromises = [
      desktopApi.onSseEvent((ev) => {
        setEvents((prev) => [ev, ...prev].slice(0, 250));
      }),
      desktopApi.onSseState(setSseState),
    ];
    return () => {
      Promise.all(unlistenPromises).then((fns) => fns.forEach((fn) => fn()));
    };
  }, []);

  const refreshOverview = async () => {
    try {
      const node = await desktopApi.fetchNodeOverview(nodeUrl);
      const tip = node.tip_height == null ? "" : ` · Tip #${node.tip_height}`;
      setOverview(`${node.status_text}${tip}`);
    } catch (err) {
      setOverview(String(err));
    }
  };

  const r7Node = r7Runtimes?.length === 1 ? r7Runtimes[0] : null;
  let r7Label = "Checking…";
  if (r7Error || !isTauriRuntime()) r7Label = "Status unavailable";
  else if (r7Node) r7Label = r7NodeLabel(r7Node);
  else if (r7Runtimes) r7Label = r7Runtimes.length ? `${r7Runtimes.length} local nodes` : "No local node";

  const openOnboarding = async () => {
    setBusy("Opening R7 onboarding…");
    setOnboardingStatus(null);
    try {
      await desktopApi.openValidatorOnboarding();
      setOnboardingStatus("R7 onboarding is open in your browser. Keep this window open while you complete the steps.");
    } catch (error) {
      setOnboardingStatus(String(error));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-badge">CREG</div>
          <div>
            <strong>Validator Console</strong>
            <div className="small muted">4-Zone Operator Desktop</div>
          </div>
        </div>
        <nav className="nav">
          {(
            [
              ["zones", "Zones & Runtime"],
              ["ai", "AI Config & Probe"],
              ["logs", "Logs & SSE v2"],
              ["evidence", "Evidence & Queue"],
              ["settings", "Settings & Paths"],
            ] as Array<[TabId, string]>
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              className={`nav-btn ${tab === id ? "nav-btn-active" : ""}`}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </nav>
      </aside>

      <div className="workspace">
        <header className="topbar">
          <div className="meta-strip">
            <span role="status">
              R7 node <strong className={r7Node?.health?.synced ? "text-ok" : "text-warn"}>{r7Label}</strong>
            </span>
            {r7Node?.health && <span>Finalized <strong>#{r7Node.health.finalized_height}</strong></span>}
            <span>
              SSE: <strong>{sseState}</strong>
            </span>
            {busy && <span className="pill">{busy}</span>}
          </div>
          <button type="button" className="btn" disabled={!isTauriRuntime()} onClick={refreshR7}>
            Refresh R7 status
          </button>
        </header>

        <main className="main">
          {!isTauriRuntime() && (
            <div className="alert alert-warn">
              Running outside the Tauri desktop shell (browser preview mode). IPC commands require launching via{" "}
              <code>npm.cmd run tauri:dev</code> or the compiled desktop binary.
            </div>
          )}

          {tab === "zones" && (
            <>
              <section className="card panel-stack">
                <h2>Join the R7 testnet</h2>
                <p className="muted">Connect a Sepolia wallet to apply or check your validator application.</p>
                {r7Error && <div className="alert alert-warn">{r7Error}</div>}
                {r7Runtimes && r7Runtimes.length > 0 && (
                  <table className="table">
                    <thead>
                      <tr>
                        <th>Local R7 node</th>
                        <th>Status</th>
                        <th>Finalized block</th>
                        <th>Admission</th>
                        <th>Approvals</th>
                        <th>Sepolia tx</th>
                        <th>Activation / Vote</th>
                      </tr>
                    </thead>
                    <tbody>
                      {r7Runtimes.map((node) => {
                        const adm = node.admission;
                        const txLabel = adm?.l1_tx_hash
                          ? `${adm.l1_tx_status} (${adm.l1_tx_hash.slice(0, 10)}…${
                              adm.l1_confirmed_block != null ? ` · #${adm.l1_confirmed_block}` : ""
                            })`
                          : (adm?.l1_tx_status ?? "—");
                        const actLabel = adm
                          ? [
                              adm.voting_activation_height != null ? `Act #${adm.voting_activation_height}` : "Act pending",
                              adm.first_finalized_vote_height != null ? `Vote #${adm.first_finalized_vote_height}` : "Vote pending",
                            ].join(" · ")
                          : "—";
                        const approvalLabel = adm
                          ? adm.l1_approvals_required > 0
                            ? `L1 ${adm.l1_approvals_collected}/${adm.l1_approvals_required}` +
                              (adm.transition_approvals_required > 0
                                ? ` · R7 ${adm.transition_approvals_collected}/${adm.transition_approvals_required}`
                                : "")
                            : adm.unavailable_reason
                              ? "Telemetry unavailable"
                              : "Pending"
                          : "—";
                        return (
                          <tr key={node.node_id}>
                            <td className="mono">{node.node_id}</td>
                            <td>{r7NodeLabel(node)}</td>
                            <td>{node.health ? `#${node.health.finalized_height}` : "—"}</td>
                            <td>
                              {adm ? (
                                <div>
                                  <strong>{adm.stage.replaceAll("_", " ")}</strong>
                                  <div className="small muted">{adm.stage_reason}</div>
                                  {adm.missing_signer_setup.length > 0 &&
                                    (adm.stage === "active_voting" ? (
                                      <div className="small muted">
                                        Optional: configure CREG_VALIDATOR_ADMISSION_KEY to co-sign future applicants
                                      </div>
                                    ) : (
                                      <div className="small text-warn">
                                        Signer: {adm.missing_signer_setup.join("; ")}
                                      </div>
                                    ))}
                                </div>
                              ) : "—"}
                            </td>
                            <td>{approvalLabel}</td>
                            <td className="mono">{txLabel}</td>
                            <td>{actLabel}</td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
                <div>
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={!isTauriRuntime() || Boolean(busy)}
                    onClick={openOnboarding}
                  >
                    Open validator onboarding
                  </button>
                </div>
                {onboardingStatus && <p role="status">{onboardingStatus}</p>}
              </section>
              <ZonesView
                env={env}
                mode={mode}
                setMode={setMode}
                composeFile={composeFile}
                supervisorBinary={supervisorBinary}
                operatorKeyFile={operatorKeyFile}
                aiConfigPath={aiConfigPath}
                runtimeStatus={runtimeStatus}
                setRuntimeStatus={setRuntimeStatus}
                doctor={doctor}
                setDoctor={setDoctor}
                busy={busy}
                setBusy={setBusy}
                errorMsg={errorMsg}
                setErrorMsg={setErrorMsg}
                hasActiveR7Node={Boolean(r7Runtimes?.some((n) => n.container_state === "running"))}
              />
            </>
          )}

          {tab === "ai" && (
            <AiConfigView
              aiConfigPath={aiConfigPath}
              setAiConfigPath={setAiConfigPath}
              operatorKeyFile={operatorKeyFile}
            />
          )}

          {tab === "logs" && (
            <LogsStreamView
              mode={mode}
              composeFile={composeFile}
              events={events}
              sseState={sseState}
              sseUrl={sseUrl}
              setSseUrl={setSseUrl}
            />
          )}

          {tab === "evidence" && (
            <EvidenceView nodeUrl={nodeUrl} indexerUrl={indexerUrl} />
          )}

          {tab === "settings" && (
            <section className="card panel-stack">
              <h2>Paths &amp; Endpoints</h2>
              <label className="field">
                Node HTTP URL
                <input className="input mono" value={nodeUrl} onChange={(e) => setNodeUrl(e.target.value)} />
              </label>
              <div className="toolbar">
                <button type="button" className="btn" disabled={!isTauriRuntime()} onClick={refreshOverview}>Ping configured node</button>
                {overview && <span role="status">{overview}</span>}
              </div>
              <label className="field">
                Indexer HTTP URL
                <input className="input mono" value={indexerUrl} onChange={(e) => setIndexerUrl(e.target.value)} />
              </label>
              <label className="field">
                Operator IPC Key Bundle (`operator-ipc-keys.json`)
                <input className="input mono" value={operatorKeyFile} onChange={(e) => setOperatorKeyFile(e.target.value)} />
              </label>
              <label className="field">
                Docker Compose File (`docker-compose.validator-v2.yml`)
                <input className="input mono" value={composeFile} onChange={(e) => setComposeFile(e.target.value)} />
              </label>
              <label className="field">
                Local Supervisor / Node Binary
                <input className="input mono" value={supervisorBinary} onChange={(e) => setSupervisorBinary(e.target.value)} />
              </label>
            </section>
          )}
        </main>
      </div>
    </div>
  );
}

export default App;
