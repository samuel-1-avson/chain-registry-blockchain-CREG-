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

const PUBLIC_R7_URL = "https://api.testnet.cregnet.dev";
const NODE_URL_SETTING = "creg.node-http-url";

function savedNodeUrl(): string {
  try {
    return window.localStorage.getItem(NODE_URL_SETTING) || PUBLIC_R7_URL;
  } catch {
    return PUBLIC_R7_URL;
  }
}

function rememberNodeUrl(url: string) {
  try {
    const endpoint = new URL(url);
    if (endpoint.username || endpoint.password || endpoint.search || endpoint.hash) return;
    window.localStorage.setItem(NODE_URL_SETTING, url);
  } catch {
    // The endpoint can still be used for this session.
  }
}

function r7NodeLabel(node: R7RuntimeStatus): string {
  if (node.health) {
    return node.health.is_validator ? "Validator reachable" : "Observer reachable";
  }
  switch (node.container_state) {
    case "running": return "R7 API unverified";
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
  const [nodeUrl, setNodeUrl] = useState(savedNodeUrl);
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
      if (node.ok) rememberNodeUrl(nodeUrl.trim());
      const chain = node.chain_id ? ` · ${node.chain_id}` : "";
      setOverview(`${node.status_text}${chain}${tip}`);
    } catch (err) {
      setOverview(String(err));
    }
  };

  const runningR7 = r7Runtimes?.filter((node) => node.container_state === "running") ?? [];
  const inactiveR7 = r7Runtimes?.filter((node) => node.container_state !== "running") ?? [];
  const verifiedR7 = runningR7.filter((node) => node.health && node.http_url);
  let r7Label = "Checking…";
  if (r7Error || !isTauriRuntime()) r7Label = "Status unavailable";
  else if (r7Runtimes) r7Label = `${runningR7.length} running · ${inactiveR7.length} inactive`;

  const selectNode = (url: string) => {
    setNodeUrl(url);
    setOverview(null);
    rememberNodeUrl(url);
  };

  const openOnboarding = async (containerName?: string) => {
    setBusy("Opening R7 onboarding…");
    setOnboardingStatus(null);
    try {
      await desktopApi.openValidatorOnboarding(containerName);
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
              R7 nodes <strong className={runningR7.length > 0 ? "text-ok" : "text-warn"}>{r7Label}</strong>
            </span>
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
                <h2>Local R7 validators</h2>
                {r7Error && <div className="alert alert-warn">{r7Error}</div>}
                {onboardingStatus && <p role="status">{onboardingStatus}</p>}
                {runningR7.length > 0 && (
                  <table className="table">
                    <thead>
                      <tr>
                        <th>Local R7 node</th>
                        <th>API</th>
                        <th>Block sync</th>
                        <th>Peers</th>
                        <th>Sepolia sync</th>
                        <th>Admission</th>
                        <th>Manage</th>
                      </tr>
                    </thead>
                    <tbody>
                      {runningR7.map((node) => {
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
                          <tr key={node.container_name}>
                            <td className="mono">{node.node_id}<div className="small muted">{node.http_url}</div></td>
                            <td>{r7NodeLabel(node)}</td>
                            <td>{node.health ? <>Tip <strong>#{node.health.tip_height}</strong><div className="small muted">
                              Finalized #{node.health.finalized_height} · {node.health.chain_healthy ? `${node.health.chain_lag_blocks} behind` : `Lagging by ${node.health.chain_lag_blocks}`}
                            </div></> : "—"}</td>
                            <td>{node.health?.peer_count ?? "—"}</td>
                            <td>{node.health?.sepolia_sync_state ?? "—"}</td>
                            <td>
                              {(node.certified_identity || adm?.certified_member) && <div className="small">Certified voting member</div>}
                              {adm ? (
                                <div>
                                  <strong>{adm.stage.replaceAll("_", " ")}</strong>
                                  {adm.registration_conflict && <div className="small text-warn">Registration does not match the certified identity</div>}
                                  <div className="small muted">{adm.stage_reason}</div>
                                  <div className="small muted">{approvalLabel} · {txLabel}</div>
                                  <div className="small muted">{actLabel}</div>
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
                              ) : node.certified_identity ? null : "—"}
                            </td>
                            <td>
                              {node.certified_identity && <details className="small">
                                <summary>Certified identity</summary>
                                <div>Wallet <code className="r7-identity-value">{node.certified_identity.evm_address}</code></div>
                                <div>Public key <code className="r7-identity-value">{node.certified_identity.ed25519_pubkey}</code></div>
                              </details>}
                              <button type="button" className="btn"
                              disabled={!node.health || Boolean(busy)}
                              onClick={() => void openOnboarding(node.container_name)}>
                              Manage existing validator
                              </button>
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
                {inactiveR7.length > 0 && (
                  <details>
                    <summary>Inactive R7 containers ({inactiveR7.length})</summary>
                    <ul>{inactiveR7.map((node) => (
                      <li key={node.container_name}><code>{node.container_name}</code> · {r7NodeLabel(node)}</li>
                    ))}</ul>
                  </details>
                )}
                <div>
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={!isTauriRuntime() || Boolean(busy)}
                    onClick={() => void openOnboarding()}
                  >
                    Enroll a new validator
                  </button>
                </div>
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
                  Verified R7 node
                  <select className="input" value={nodeUrl === PUBLIC_R7_URL || verifiedR7.some((node) => node.http_url === nodeUrl) ? nodeUrl : ""}
                    onChange={(event) => { if (event.target.value) selectNode(event.target.value); }}>
                    <option value="">Custom endpoint</option>
                    <option value={PUBLIC_R7_URL}>Public R7 observer</option>
                    {verifiedR7.map((node) => (
                      <option key={node.container_name} value={node.http_url ?? ""}>{node.node_id} · {node.http_url}</option>
                    ))}
                  </select>
                </label>
              <label className="field">
                Node HTTP URL
                <input className="input mono" value={nodeUrl} onChange={(e) => { setNodeUrl(e.target.value); setOverview(null); }} />
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
