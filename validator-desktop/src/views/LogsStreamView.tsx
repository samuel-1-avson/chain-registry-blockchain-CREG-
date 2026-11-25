import { useEffect, useState } from "react";
import { desktopApi, type RuntimeMode, type SseEnvelopeV2 } from "../api/tauri";
import { UntrustedText } from "../components/UntrustedText";

export function LogsStreamView({
  mode,
  composeFile,
  events,
  sseState,
  sseUrl,
  setSseUrl,
}: {
  mode: RuntimeMode;
  composeFile: string;
  events: SseEnvelopeV2[];
  sseState: string;
  sseUrl: string;
  setSseUrl: (u: string) => void;
}) {
  const [tab, setTab] = useState<"logs" | "sse">("logs");
  const [logs, setLogs] = useState<string[]>([]);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const fetchLogs = async () => {
    try {
      const lines = await desktopApi.getRuntimeLogs(mode, composeFile || undefined, 250);
      setLogs(lines);
      setErrorMsg(null);
    } catch (err) {
      setErrorMsg(String(err));
    }
  };

  useEffect(() => {
    if (tab !== "logs" || !autoRefresh) return;
    fetchLogs();
    const timer = setInterval(fetchLogs, 4000);
    return () => clearInterval(timer);
  }, [tab, autoRefresh, mode, composeFile]);

  const toggleStream = async () => {
    setErrorMsg(null);
    try {
      if (sseState === "live" || sseState === "connecting") {
        await desktopApi.stopSseStream();
      } else {
        await desktopApi.startSseStream(sseUrl);
      }
    } catch (err) {
      setErrorMsg(String(err));
    }
  };

  return (
    <div className="panel-stack">
      <div className="tabs">
        <button
          type="button"
          className={`tab ${tab === "logs" ? "tab-active" : ""}`}
          onClick={() => setTab("logs")}
        >
          Runtime Logs
        </button>
        <button
          type="button"
          className={`tab ${tab === "sse" ? "tab-active" : ""}`}
          onClick={() => setTab("sse")}
        >
          Live SSE v2 Events ({events.length})
        </button>
      </div>

      {tab === "logs" ? (
        <section className="card">
          <div className="toolbar">
            <button type="button" className="btn btn-primary" onClick={fetchLogs}>
              Refresh Logs
            </button>
            <label className="inline-label">
              <input
                type="checkbox"
                checked={autoRefresh}
                onChange={(e) => setAutoRefresh(e.target.checked)}
              />
              Auto-refresh (4s)
            </label>
          </div>
          {errorMsg && <div className="alert alert-warn">{errorMsg}</div>}
          <pre className="log-box mono">
            {logs.length > 0 ? logs.join("\n") : "No runtime logs captured yet."}
          </pre>
        </section>
      ) : (
        <section className="card">
          <div className="toolbar">
            <input
              type="text"
              className="input grow mono"
              value={sseUrl}
              onChange={(e) => setSseUrl(e.target.value)}
            />
            <span className={`pill ${sseState === "live" ? "pill-ok" : "pill-warn"}`}>
              {sseState}
            </span>
            <button type="button" className="btn btn-primary" onClick={toggleStream}>
              {sseState === "live" || sseState === "connecting" ? "Disconnect" : "Connect SSE"}
            </button>
          </div>
          {errorMsg && <div className="alert alert-err">{errorMsg}</div>}
          <div className="event-list">
            {events.length === 0 ? (
              <p className="muted">No SSE v2 events received yet.</p>
            ) : (
              events.map((ev) => (
                <div key={`${ev.id}-${ev.kind}`} className="event-row">
                  <span className="pill">{ev.kind}</span>
                  <span className="mono small">#{ev.id}</span>
                  {ev.canonical && <UntrustedText value={ev.canonical} maxGraphemes={96} />}
                </div>
              ))
            )}
          </div>
        </section>
      )}
    </div>
  );
}
