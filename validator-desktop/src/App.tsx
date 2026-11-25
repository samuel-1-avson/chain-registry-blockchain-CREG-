import { useEffect, useState } from "react";
import {
  desktopApi,
  isTauriRuntime,
  type DesktopDoctorReport,
  type EnvironmentInfo,
  type NodeOverview,
  type RuntimeMode,
  type RuntimeStatus,
  type SseEnvelopeV2,
} from "./api/tauri";
import { ZonesView } from "./views/ZonesView";
import { AiConfigView } from "./views/AiConfigView";
import { LogsStreamView } from "./views/LogsStreamView";
import { EvidenceView } from "./views/EvidenceView";

type TabId = "zones" | "ai" | "logs" | "evidence" | "settings";

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
  const [doctor, setDoctor] = useState<DesktopDoctorReport | null>(null);
  const [overview, setOverview] = useState<NodeOverview | null>(null);
  const [events, setEvents] = useState<SseEnvelopeV2[]>([]);
  const [sseState, setSseState] = useState("idle");
  const [busy, setBusy] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

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
      setOverview(await desktopApi.fetchNodeOverview(nodeUrl));
    } catch (err) {
      setErrorMsg(String(err));
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
            <span>
              Runtime:{" "}
              <strong className={runtimeStatus?.running ? "text-ok" : "text-warn"}>
                {runtimeStatus?.running ? "Running" : "Stopped"}
              </strong>
            </span>
            <span>
              Tip: <strong>{overview?.tip_height != null ? `#${overview.tip_height}` : "--"}</strong>
            </span>
            <span>
              SSE: <strong>{sseState}</strong>
            </span>
            {busy && <span className="pill">{busy}</span>}
          </div>
          <button type="button" className="btn" onClick={refreshOverview}>
            Ping Node
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
            />
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
